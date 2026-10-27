# Local patches to egui 0.36.2 / epaint 0.36.2

egui has no bidirectional text support yet. These patches add it (modelled on gedit/GTK):

- `epaint/src/text/bidi.rs`: UAX #9 analysis (via `unicode-bidi`) and reordering (rule L2).
- `epaint/src/text/text_layout.rs`: shaping per direction run, RTL runs kept pixel-exact, row-wise visual reordering (L1/L2), right-aligned RTL paragraphs, bidi-aware line breaking. Tests at the bottom of the file.
- `epaint/src/text/text_layout_types.rs`: `Glyph::bidi_level`, `Row::base_level`, caret x-positions / hit-testing / visual left-right movement.
- `egui/src/text_selection/visuals.rs`: selection rectangle for reordered rows.

Pure left-to-right text takes the original code path. Delete `vendor/` and the `[patch.crates-io]` section of Cargo.toml to go back to stock egui.
