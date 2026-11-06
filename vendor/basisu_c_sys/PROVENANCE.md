# Basis Universal C bindings

Vendored from crates.io `basisu_c_sys` 0.9.1 (MIT OR Apache-2.0), including
its Basis Universal sources (Apache-2.0; copyright headers retained).
Upstream: https://github.com/beicause/basisu_c_sys

Local changes:
- `BU_COMP_FLAGS_NO_STATS` leaves the C wrapper statistics pointer null.
- `BU_COMP_FLAGS_NO_UASTC_HINTS` skips ETC1/ETC2/BC1 hint generation when
  encoding UASTC for ASTC or BC7. The intermediate UASTC hint bits differ;
  the final ASTC and BC7 blocks remain identical.
- The internal compression entry point preserves 64-bit flags so these
  opt-outs do not reuse upstream's existing 32-bit flag assignments.

These flags are opt-in; unmodified callers retain upstream behavior. Do not
use the hint opt-out for ETC/BC1 output or UASTC RDO.
