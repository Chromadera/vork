# Text engine provenance

Bidi visual runs, script itemization and the shaping loop are adapted from
moda-text's `src/vector/hb.rs` (MIT OR Apache-2.0). The original licence texts
are retained beside this file.



This lift uses MIT HarfRust 0.15 and the maintained xberg-ttf-parser.
Shape-plan accounting and patched parser APIs from the source engine are not
needed by the authoring-time builder.

The Noto Sans Arabic and Devanagari test fonts are copied from the source
checkout's `examples/showcase/fonts/`; their SIL OFL licence is in
`tests/fonts/OFL.txt`.
