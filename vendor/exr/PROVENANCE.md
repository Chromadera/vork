# exr 1.74.2

Source: the unmodified crates.io `exr` 1.74.2 package, except for the local
change below and this file. Registry metadata `.cargo-ok` and
`.cargo_vcs_info.json`, `Cargo.toml.orig`, `.github/` and `.gitignore` are
omitted.
Upstream: https://github.com/johannesvollmer/exrs. Licensed BSD-3-Clause;
LICENSE.md is retained. The repository's `.gitattributes` stores the
package's CRLF text files with LF line endings.

Local patch: `src/compression/dwa/mod.rs`, `decompress` checks the DWA chunk
header's counts (`check_counts`) against what the chunk's channels can hold
before any section buffer is allocated. Upstream sizes the AC Huffman output from
`ac_count` unchecked, so a 753-byte file asked for 562 TB and aborted the process
(`Vec::with_capacity` in `piz/huffman.rs`), and a larger count overflowed capacity
and panicked. Remove when upstream bounds these counts.
