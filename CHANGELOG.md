# v0.3.5 - August 12th, 2019

- Support for haunted and rugby games.
- Improvement to error handling that gives detailed error messages on what new / updated actor may have received changes in the RL update. These error messages should only be helpful debugging new updates.
- Several security fixes:
  - Malicious user could craft NumFrames property to be obscenely high and run the machine out of memory. An error is now thrown if the requested number of frames is greater than the number of bytes remaining.
  - A class's network cache that referenced an out of range object id would cause a index out of bound panic. Now an error is raised.
  - Other fixes are for panics in debug builds

# v0.3.4 - June 5th, 2019

* Update network parser to be compatible with v1.63 rocket league replays

# v0.3.3 - June 1st, 2019

- Update crc content from signed 32bits to unsigned 32bits as a negative checksum can be misleading.
- Additional decoding for PsyNet, Switch, and Ps4 remote ids. Instead of just a vector of opaque bytes, now the values contain a structure with additional fields like (`online_id` or `name`). Any leftover data is still captured as opaque bytes.

# v0.3.2 - May 24th 2019

- Update multimap requirement from 0.4 to 0.5
- Bugfix for newer replays with reservations involving psynet players

# v0.3.1 - May 23rd 2019

- Fix compilation edge case
- Update if_chain requirement from 0.1 to 1.0

# v0.3.0 - May 2nd 2019

* Minor version bump as the network API grew significantly. A lot of the network attributes were publicly opaque, so while one could access all the members (and write them out as JSON for instance) there was no way to access individual fields on these attributes (like RigidBody::sleeping was inaccessible). Hiding these fields was an oversight and has been fixed.
* Update encoding_rs from 0.7 to 0.8 (no discernible changes should be expected)

# v0.2.8 - April 25th 2019

* Serialize 64bit numbers as strings, so that JSON parsers don't lose any data
  in parsing them as 64bit floating point
  * Javascript numbers are 64bit floating point. 64bit integers can't be
    represented wholly in floating point notation. Thus serialize them as
    strings so that downstream applications can decide on how best to interpret
    large numbers (like 76561198122624102). Affects Int64, QWord, Steam, and
    XBox attributes.
* QWord header property changes from i64 to u64 as some pointed out that
  negative numbers didn't make sense for QWord properties (OnlineId)

# v0.2.7 - April 22nd 2019

* Update network parser to be compatible with v1.61 rocket league replays

# v0.2.6 - April 4th 2019

* Update network parser to be compatible with v1.59 rocket league replays

# v0.2.5 - September 6th 2018

* Update network parser to be compatible with v1.50 rocket league replays

# v0.2.4 - May 30th, 2018

* Update network parser to be compatible with v1.45 rocket league replays

# v0.2.3 - April 25th, 2018

* Update network parser to be compatible with latest rocket league replays
* Improve throughput of network parsing by up to 10%
* Additional detailed error messages

# v0.2.2 - March 18th, 2018

* Update network parser to the latest rocket league replays

# v0.2.1 - February 14th, 2018

* Fixed several bugs surrounding parsing of the network data. More replays are now parseable

# v0.2.0 - January 31st, 2018

Initial release of the boxcars Rust library. v0.1.0 was never released on crates.io, but was used transitively with v0.1.0 of rrrocket (hence the initial version being v0.2.0 instead of v0.1.0)
