//! # Parsing
//!
//! A Rocket League game replay is a little endian binary encoded file with an emphasis. The number
//! 100 would be represented as the four byte sequence:
//!
//! ```plain
//! 0x64 0x00 0x00 0x00
//! ```
//!
//! This in contrast to big-endian, which would represent the number as:
//!
//! ```plain
//! 0x00 0x00 0x00 0x64
//! ```
//!
//! A replay is split into three major sections, a header, body, and footer.
//!
//! ## Header
//!
//! The first four bytes of a replay is the number of bytes that comprises the header. A length
//! prefixed integer is very common throughout a replay. This prefix may either be in reference to
//! the number of bytes an elements takes up, as just seen, or the number of elements in a list.
//!
//! The next four bytes make up the [cyclic redundancy check
//! (CRC)](https://en.wikipedia.org/wiki/Cyclic_redundancy_check) for the header. The check ensures
//! that the data has not be tampered with or, more likely, corrupted.
//!
//! The game's major and minor version follow, each 32bit integers.
//!
//! Subsequently, the game type is encoded as a string. Strings in Rocket League Replay files are
//! length prefixed and null terminated.
//!
//! The properties is where all the good nuggets of info reside. Visualize the properties as a map
//! of strings to various types (number, string, array) that continues until a "None" key is found.
//!
//! ## Body
//!
//! Out of the body we get:
//!
//! - Levels (what level did the match take place)
//! - `KeyFrames`
//! - The body's crc. This check is actually for the rest of the content (including the footer).
//!
//! Since everything is length prefixed, we're able to skip the network stream data. This would be
//! 90% of the file.  Most of the interesting bits like player stats and goals are contained in the
//! header, so it's not a tremendous loss if we can't parse the network data.
//!
//! ## Footer
//!
//! After the network stream there we see:
//!
//! - Debug info
//! - Tickmarks
//! - Packages
//! - Etc

use crate::core_parser::CoreParser;
use crate::crc::calc_crc;
use crate::errors::ParseError;
use crate::header::{self, Header};
use crate::models::*;
use crate::network;
use crate::parsing_utils::{le_f32, le_i32};
use failure::{Error, ResultExt};
use std::borrow::Cow;

/// Determines under what circumstances the parser should perform the crc check for replay
/// corruption. Since the crc check is the most time consuming check for parsing (causing
/// microseconds to turn into milliseconds), clients should choose under what circumstances a crc
/// check is performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrcCheck {
    /// Always perform the crc check. Useful when the replay has had its contents modified. This
    /// will catch a user that increased the number of goals they scored (easy) but only if they
    /// didn't update the crc as well (not as easy).
    Always,

    /// Never perform the crc check. Useful only when it doesn't matter to know if a replay is
    /// corrupt or not, you either want the data or the parsing error.
    Never,

    /// Only perform the crc check when parsing a section fails. This option gets the best of both
    /// worlds. If parsing fails, the crc check will determine if it is a programming error or the
    /// replay is corrupt. If parsing succeeds it won't precious time performing the check. This
    /// option is the default for parsing.
    OnError,
}

/// Determines how the parser should handle the network data, which is the most
/// intensive and volatile section of the replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkParse {
    /// If the network data fails parse return an error
    Always,

    /// Skip parsing the network data
    Never,

    /// Attempt to parse the network data, but if unsuccessful ignore the error
    /// and continue parsing
    IgnoreOnError,
}

/// The main entry point to parsing replays in boxcars. Allows one to customize parsing options,
/// such as only parsing the header and forgoing crc (corruption) checks.
#[derive(Debug, Clone, PartialEq)]
pub struct ParserBuilder<'a> {
    data: &'a [u8],
    crc_check: Option<CrcCheck>,
    network_parse: Option<NetworkParse>,
}

impl<'a> ParserBuilder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        ParserBuilder {
            data,
            crc_check: None,
            network_parse: None,
        }
    }

    pub fn always_check_crc(mut self) -> ParserBuilder<'a> {
        self.crc_check = Some(CrcCheck::Always);
        self
    }

    pub fn never_check_crc(mut self) -> ParserBuilder<'a> {
        self.crc_check = Some(CrcCheck::Never);
        self
    }

    pub fn on_error_check_crc(mut self) -> ParserBuilder<'a> {
        self.crc_check = Some(CrcCheck::OnError);
        self
    }

    pub fn with_crc_check(mut self, check: CrcCheck) -> ParserBuilder<'a> {
        self.crc_check = Some(check);
        self
    }

    pub fn must_parse_network_data(mut self) -> ParserBuilder<'a> {
        self.network_parse = Some(NetworkParse::Always);
        self
    }

    pub fn never_parse_network_data(mut self) -> ParserBuilder<'a> {
        self.network_parse = Some(NetworkParse::Never);
        self
    }

    pub fn ignore_network_data_on_error(mut self) -> ParserBuilder<'a> {
        self.network_parse = Some(NetworkParse::IgnoreOnError);
        self
    }

    pub fn with_network_parse(mut self, parse: NetworkParse) -> ParserBuilder<'a> {
        self.network_parse = Some(parse);
        self
    }

    pub fn parse(self) -> Result<Replay<'a>, Error> {
        let mut parser = Parser::new(
            self.data,
            self.crc_check.unwrap_or(CrcCheck::OnError),
            self.network_parse.unwrap_or(NetworkParse::IgnoreOnError),
        );
        parser.parse()
    }
}

/// Intermediate parsing structure for the body / footer
#[derive(Debug, PartialEq)]
pub struct ReplayBody<'a> {
    pub levels: Vec<Cow<'a, str>>,
    pub keyframes: Vec<KeyFrame>,
    pub debug_info: Vec<DebugInfo<'a>>,
    pub tick_marks: Vec<TickMark<'a>>,
    pub packages: Vec<Cow<'a, str>>,
    pub objects: Vec<Cow<'a, str>>,
    pub names: Vec<Cow<'a, str>>,
    pub class_indices: Vec<ClassIndex<'a>>,
    pub net_cache: Vec<ClassNetCache>,
    pub network_data: &'a [u8],
}

/// Holds the current state of parsing a replay
#[derive(Debug, Clone, PartialEq)]
pub struct Parser<'a> {
    core: CoreParser<'a>,
    crc_check: CrcCheck,
    network_parse: NetworkParse,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8], crc_check: CrcCheck, network_parse: NetworkParse) -> Self {
        Parser {
            core: CoreParser::new(data),
            crc_check,
            network_parse,
        }
    }

    fn err_str(&self, desc: &'static str, e: &ParseError) -> String {
        format!(
            "Could not decode replay {} at offset ({}): {}",
            desc,
            self.core.bytes_read(),
            e
        )
    }

    fn parse(&mut self) -> Result<Replay<'a>, Error> {
        let header_size = self
            .core
            .take(4, le_i32)
            .with_context(|e| self.err_str("header size", e))?;

        let header_crc = self
            .core
            .take(4, le_i32)
            .map(|x| x as u32)
            .with_context(|e| self.err_str("header crc", e))?;

        let header_data = self
            .core
            .view_data(header_size as usize)
            .with_context(|e| self.err_str("header data", e))?;

        let header =
            self.crc_section(header_data, header_crc as u32, "header", Self::parse_header)?;

        let content_size = self
            .core
            .take(4, le_i32)
            .with_context(|e| self.err_str("content size", e))?;

        let content_crc = self
            .core
            .take(4, le_i32)
            .map(|x| x as u32)
            .with_context(|e| self.err_str("content crc", e))?;

        let content_data = self
            .core
            .view_data(content_size as usize)
            .with_context(|e| self.err_str("content data", e))?;

        let body = self.crc_section(content_data, content_crc as u32, "body", Self::parse_body)?;

        let mut network: Option<NetworkFrames> = None;
        match self.network_parse {
            NetworkParse::Always => {
                network = Some(self.parse_network(&header, &body)?);
            }
            NetworkParse::IgnoreOnError => {
                if let Ok(v) = self.parse_network(&header, &body) {
                    network = Some(v);
                }
            }
            NetworkParse::Never => network = None,
        }

        Ok(Replay {
            header_size,
            header_crc,
            major_version: header.major_version,
            minor_version: header.minor_version,
            net_version: header.net_version,
            game_type: header.game_type,
            properties: header.properties,
            content_size,
            content_crc,
            network_frames: network,
            levels: body.levels,
            keyframes: body.keyframes,
            debug_info: body.debug_info,
            tick_marks: body.tick_marks,
            packages: body.packages,
            objects: body.objects,
            names: body.names,
            class_indices: body.class_indices,
            net_cache: body.net_cache,
        })
    }

    fn parse_network(
        &mut self,
        header: &Header<'_>,
        body: &ReplayBody<'_>,
    ) -> Result<NetworkFrames, Error> {
        network::parse(header, body)
    }

    fn parse_header(&mut self) -> Result<Header<'a>, Error> {
        header::parse_header(&mut self.core)
    }

    /// Parses a section and performs a crc check as configured
    fn crc_section<T, F>(
        &mut self,
        data: &[u8],
        crc: u32,
        section: &str,
        mut f: F,
    ) -> Result<T, Error>
    where
        F: FnMut(&mut Self) -> Result<T, Error>,
    {
        match (self.crc_check, f(self)) {
            (CrcCheck::Always, res) => {
                let actual = calc_crc(data);
                if actual != crc as u32 {
                    Err(Error::from(ParseError::CrcMismatch(crc, actual)))
                } else {
                    res
                }
            }
            (CrcCheck::OnError, Err(e)) => {
                let actual = calc_crc(data);
                if actual != crc as u32 {
                    Err(e
                        .context(format!(
                            "Failed to parse {} and crc check failed. Replay is corrupt",
                            section
                        ))
                        .into())
                } else {
                    Err(e)
                }
            }
            (CrcCheck::OnError, Ok(s)) => Ok(s),
            (CrcCheck::Never, res) => res,
        }
    }

    fn parse_body(&mut self) -> Result<ReplayBody<'a>, Error> {
        let levels = self
            .core
            .text_list()
            .with_context(|e| self.err_str("levels", e))?;

        let keyframes = self
            .parse_keyframe()
            .with_context(|e| self.err_str("keyframes", e))?;

        let network_size = self
            .core
            .take(4, le_i32)
            .with_context(|e| self.err_str("network size", e))?;

        let network_data = self
            .core
            .take(network_size as usize, |d| d)
            .with_context(|e| self.err_str("network data", e))?;

        let debug_infos = self
            .parse_debuginfo()
            .with_context(|e| self.err_str("debug info", e))?;

        let tickmarks = self
            .parse_tickmarks()
            .with_context(|e| self.err_str("tickmarks", e))?;

        let packages = self
            .core
            .text_list()
            .with_context(|e| self.err_str("packages", e))?;
        let objects = self
            .core
            .text_list()
            .with_context(|e| self.err_str("objects", e))?;
        let names = self
            .core
            .text_list()
            .with_context(|e| self.err_str("names", e))?;

        let class_index = self
            .parse_classindex()
            .with_context(|e| self.err_str("class index", e))?;

        let net_cache = self
            .parse_classcache()
            .with_context(|e| self.err_str("net cache", e))?;

        Ok(ReplayBody {
            levels,
            keyframes,
            debug_info: debug_infos,
            tick_marks: tickmarks,
            packages,
            objects,
            names,
            class_indices: class_index,
            net_cache,
            network_data,
        })
    }

    fn parse_tickmarks(&mut self) -> Result<Vec<TickMark<'a>>, ParseError> {
        self.core.list_of(|s| {
            Ok(TickMark {
                description: s.parse_text()?,
                frame: s.take(4, le_i32)?,
            })
        })
    }

    fn parse_keyframe(&mut self) -> Result<Vec<KeyFrame>, ParseError> {
        self.core.list_of(|s| {
            Ok(KeyFrame {
                time: s.take(4, le_f32)?,
                frame: s.take(4, le_i32)?,
                position: s.take(4, le_i32)?,
            })
        })
    }

    fn parse_debuginfo(&mut self) -> Result<Vec<DebugInfo<'a>>, ParseError> {
        self.core.list_of(|s| {
            Ok(DebugInfo {
                frame: s.take(4, le_i32)?,
                user: s.parse_text()?,
                text: s.parse_text()?,
            })
        })
    }

    fn parse_classindex(&mut self) -> Result<Vec<ClassIndex<'a>>, ParseError> {
        self.core.list_of(|s| {
            Ok(ClassIndex {
                class: s.parse_str()?,
                index: s.take(4, le_i32)?,
            })
        })
    }

    fn parse_classcache(&mut self) -> Result<Vec<ClassNetCache>, ParseError> {
        self.core.list_of(|x| {
            Ok(ClassNetCache {
                object_ind: x.take(4, le_i32)?,
                parent_id: x.take(4, le_i32)?,
                cache_id: x.take(4, le_i32)?,
                properties: x.list_of(|s| {
                    Ok(CacheProp {
                        object_ind: s.take(4, le_i32)?,
                        stream_id: s.take(4, le_i32)?,
                    })
                })?,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TickMark;
    use std::borrow::Cow;

    #[test]
    fn key_frame_list() {
        let data = include_bytes!("../assets/replays/good/rumble.replay");

        // List is 2A long, each keyframe is 12 bytes. Then add four for list length = 508
        let mut parser = Parser::new(
            &data[0x12ca..0x12ca + 508],
            CrcCheck::Never,
            NetworkParse::Never,
        );
        let frames = parser.parse_keyframe().unwrap();
        assert_eq!(frames.len(), 42);
    }

    #[test]
    fn tickmark_list() {
        let data = include_bytes!("../assets/replays/good/rumble.replay");

        // 7 tick marks at 8 bytes + size of tick list
        let mut parser = Parser::new(
            &data[0xf6cce..0xf6d50],
            CrcCheck::Never,
            NetworkParse::Never,
        );
        let ticks = parser.parse_tickmarks().unwrap();

        assert_eq!(ticks.len(), 7);
        assert_eq!(
            ticks[0],
            TickMark {
                description: Cow::Borrowed("Team1Goal"),
                frame: 396,
            }
        );
    }

    #[test]
    fn test_the_parsing_empty() {
        let mut parser = Parser::new(&[], CrcCheck::Never, NetworkParse::Never);
        assert!(parser.parse().is_err());
    }

    #[test]
    fn test_the_parsing_text_too_long() {
        let data = include_bytes!("../assets/replays/bad/fuzz-string-too-long.replay");
        let mut parser = Parser::new(&data[..], CrcCheck::Never, NetworkParse::Never);
        assert!(parser.parse().is_err())
    }

    #[test]
    fn test_the_parsing_text_too_long2() {
        let data = include_bytes!("../assets/replays/bad/fuzz-string-too-long2.replay");
        let mut parser = Parser::new(&data[..], CrcCheck::Never, NetworkParse::Always);
        let err = parser.parse().unwrap_err();
        assert_eq!(
            "Attribute error: Unexpected size for string: -1912602609",
            format!("{}", err)
        );
    }

    #[test]
    fn test_fuzz_corpus_slice_index() {
        let data = include_bytes!("../assets/replays/bad/fuzz-slice-index.replay");
        let mut parser = Parser::new(&data[..], CrcCheck::Never, NetworkParse::Never);
        assert!(parser.parse().is_err())
    }

    #[test]
    fn test_the_fuzz_corpus_abs_panic() {
        let data = include_bytes!("../assets/replays/bad/fuzz-corpus.replay");
        let mut parser = Parser::new(&data[..], CrcCheck::Never, NetworkParse::Never);
        assert!(parser.parse().is_err())
    }

    #[test]
    fn test_the_fuzz_corpus_large_list() {
        let data = include_bytes!("../assets/replays/bad/fuzz-list-too-large.replay");
        let mut parser = Parser::new(&data[..], CrcCheck::Never, NetworkParse::Never);
        let err = parser.parse().unwrap_err();
        assert!(format!("{}", err)
            .starts_with("Could not decode replay debug info at offset (1010894): list of size",));
    }

    #[test]
    fn test_the_fuzz_corpus_large_list_on_error_crc() {
        let data = include_bytes!("../assets/replays/bad/fuzz-list-too-large.replay");
        let mut parser = Parser::new(&data[..], CrcCheck::OnError, NetworkParse::Never);
        let err = parser.parse().unwrap_err();
        assert_eq!(
            "Failed to parse body and crc check failed. Replay is corrupt",
            format!("{}", err)
        );

        assert!(format!("{}", err.as_fail().cause().unwrap())
            .starts_with("Could not decode replay debug info at offset (1010894): list of size",));
    }

    #[test]
    fn test_the_fuzz_corpus_large_list_always_crc() {
        let data = include_bytes!("../assets/replays/bad/fuzz-list-too-large.replay");
        let mut parser = Parser::new(&data[..], CrcCheck::Always, NetworkParse::Never);
        let err = parser.parse().unwrap_err();
        assert_eq!(
            "Crc mismatch. Expected 3765941959 but received 1314727725",
            format!("{}", err)
        );
        assert!(err.as_fail().cause().is_none());
    }

    #[test]
    fn test_the_fuzz_object_id_too_large() {
        let data = include_bytes!("../assets/replays/bad/fuzz-large-object-id.replay");
        let mut parser = Parser::new(&data[..], CrcCheck::Never, NetworkParse::Always);
        let err = parser.parse().unwrap_err();
        assert_eq!(
            "Object Id of 1547 exceeds range",
            format!("{}", err)
        );
        assert!(err.as_fail().cause().is_none());
    }

    #[test]
    fn test_the_fuzz_too_many_frames() {
        let data = include_bytes!("../assets/replays/bad/fuzz-too-many-frames.replay");
        let mut parser = Parser::new(&data[..], CrcCheck::Never, NetworkParse::Always);
        let err = parser.parse().unwrap_err();
        assert_eq!(
            "Too many frames to decode: 738197735",
            format!("{}", err)
        );
        assert!(err.as_fail().cause().is_none());
    }

    #[test]
    fn test_crc_check_with_bad() {
        let mut data = include_bytes!("../assets/replays/good/rumble.replay").to_vec();

        // Changing this byte won't make the parsing fail but will make the crc check fail
        data[4775] = 100;
        let mut parser = Parser::new(&data[..], CrcCheck::Always, NetworkParse::Never);
        let res = parser.parse();
        assert!(res.is_err());
        assert_eq!(
            "Crc mismatch. Expected 337843175 but received 2877465516",
            format!("{}", res.unwrap_err())
        );

        parser = Parser::new(&data[..], CrcCheck::OnError, NetworkParse::Never);
        assert!(parser.parse().is_ok());
    }
}
