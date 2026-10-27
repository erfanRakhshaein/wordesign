# wordesign

A desktop editor for writing AI prompts, built in Rust with egui/eframe. Dark violet theme with neon purple borders.

## Build & run

Needs a current Rust toolchain (the dependencies use edition 2024, so rustc 1.95+; check with `rustc --version`). Install or update via [rustup](https://rustup.rs): `rustup update stable`.

Linux also needs the usual windowing headers, e.g. on Debian/Ubuntu:

```bash
sudo apt install pkg-config libx11-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libxrandr-dev libxi-dev libxcursor-dev libxfixes-dev libgl-dev libegl-dev
```

```bash
cargo run --release
```

## How it works

- **Starts as an empty note.** No headers. Use the toolbar to add a Context, Definitions, Rules or a mermaid Flow whenever you need them.
- **Autosave.** Every prompt is one `.aiprompt` file in `~/.wordesign` (override with the `WORDESIGN_DIR` env var). New prompts are named `prompt_1`, `prompt_2`, … unless you rename them. Empty notes are never written to disk.
- **Ctrl+Enter** submits what you are editing (collapses a flow, adds an example); anywhere else it toggles the prompt preview.
- **Benchmarks tab.** Per prompt, store test questions with several answers, each tagged with the model that produced it (Q1: a(model 1) / a(model 2) …). Benchmarks are saved with the prompt but never appear in the preview or the export.
- **Folders.** Create folders with **+ Folder** in the sidebar and drag any prompt in the sidebar onto a folder (or onto **No folder**) to file it, or use the **+** on a folder to start a prompt inside it. Folders are real sub-directories of `~/.wordesign` (one level), so they work with any file manager or git.
- **Flowcharts in the preview.** Mermaid blocks are drawn as real diagrams in the preview window. The export still contains the plain mermaid notation.
- **Unicode.** Files are UTF-8 throughout. Chinese, Japanese, Korean, Arabic, Hebrew, Thai, Devanagari etc. are loaded from your system fonts automatically. Prompt and folder names may be in any script.
- **Handwriting-style font.** The UI uses the bundled [Patrick Hand](https://fonts.google.com/specimen/Patrick+Hand) (SIL OFL, see `assets/fonts/OFL.txt`).
- **Right-to-left text** (Arabic, Persian, Hebrew…) works like in gedit: each paragraph's direction comes from its first strong letter, RTL paragraphs are right-aligned, mixed text is reordered, and the arrow keys move visually. This uses a patched copy of egui in `vendor/` (see `vendor/PATCHES.md`).
- **Window.** Starts maximized at the zoom you get from pressing Ctrl+Plus five times (Ctrl+Plus / Ctrl+Minus still adjust it).
- **Preview** (Ctrl+Enter) is an overlay covering 80% of the window; it renders the whole prompt as markdown, with flowcharts drawn as diagrams. RTL paragraphs get their list bullets on the right.
- **Spell check.** In the preview, misspelled words are red; click one for suggestions ("Replace all", "Add to dictionary"), or press **Fix N misspelled** to apply the best suggestion everywhere (prose only: code, URLs, `{{placeholders}}`, acronyms, numbers and non-Latin scripts are left alone). English (en_US) dictionary bundled; your own words are kept in `~/.wordesign/.personal_dictionary.txt`.
- **Defined terms.** A word the dictionary does not know that matches a term from the Definitions section shows in cyan; click it to read the definition (and jump to it in the editor).
- **Benchmarks:** each question has a **Copy** button.
- **Menu bar.** *File* (new prompt, export, preview) and *Settings*: *Appearance > Theme color* (presets or any custom color; the whole UI re-tints, violet is the default) and *General* (after how many days of inactivity the temp folder is emptied: 1–30 days or never, default 3; or empty it right away). Settings live in `~/.wordesign/.settings`.
- **temp folder.** A built-in folder (amber italic name) always listed last. Prompts in it that have not been modified for the chosen interval are deleted on startup and every ten minutes; the open prompt is never deleted. Right-click it for *Empty temp now*. It cannot be deleted.
- **Code blocks.** `+ Code` adds a block to the main section, like a flow: paste code, Ctrl+Enter collapses it to one line. It is exported inside a ``` fence (a longer fence if the code contains backticks; already-fenced pastes are not fenced twice). Optional title is exported in bold above it.
- **Variables.** Write `{{name}}` (letters, digits, `_ - .`) in the Context, Definitions, Main text or Rules; Alt+V types `{{}}` with the cursor inside. Each name gets a card in the **Variables** tab (red count in the tab while values are missing). Ctrl+Enter collapses a filled card to a one-line chip. In the preview the tags are cyan and clickable (jump to the value); export replaces every tag with its value. Export is blocked while a used variable has no value, and the preview shows **You have unaddressed variables** in bold red. `{{…}}` inside flows and code blocks is left alone (mermaid hexagons, templates).
- **Delete a prompt or folder:** right-click it; permanent, with confirmation.
- **Delete a folder:** right-click it, *Delete folder…*, confirm. This permanently deletes the folder and the prompts inside.
- **Shortcuts.** Ctrl+E exports (also inside the preview). Alt+Up / Alt+Down moves the current line, or the selected lines, up or down in any multiline editor.
- **Benchmarks:** each question can have an optional *Expected answer* (stored in the file, never exported or previewed) to review model answers against later.
- **New prompt (Ctrl+N):** creates it, selects its name box (type the name, Enter) and then moves on to the body.
- **Find & replace (Ctrl+F / Ctrl+H, F3 / Shift+F3):** plain text or regex (`.*` toggle), optional match case. Searches the tab you are on (editor, variable values or benchmarks), or the pad when it is open. In regex mode the replacement understands `\n`, `\t` and `$1`, so find `,\s*` and replace with `\n` turns a comma list into lines. Replace one / Replace all.
- **Pad (Ctrl+P):** a scratch window to paste and pre-edit text (find & replace works inside). Ctrl+Enter inserts it where your cursor was in the prompt; Esc closes it but keeps the draft.
- **Cheatsheet:** a normal prompt in "no folder", opened in the preview on first launch; Help > Cheatsheet brings it back.
- **Version control:** tick **track** next to the name (not available in `temp`) to keep that prompt's history in a local git repository in `~/.wordesign` (needs `git` installed). Ctrl+K commits that prompt only, asking for a note. Right-click a tracked prompt > Rollback lists earlier versions; restoring first saves your current state, then records the restore as a new version. Renaming or moving a tracked prompt keeps its history. Status bar shows committed / uncommitted.
- **Sidebar:** folders start collapsed, folder names are bold and prompts are indented under them with a guide line.
- **Export** compiles the prompt to markdown (H1 per added section, your own headers demoted to H2), strips visual-only whitespace and copies the minimal result to the clipboard.

## Install on Ubuntu

```
unzip wordesign-v1.1.zip && cd wordesign
./install.sh
```

Puts `wordesign` in `~/.local/bin`, adds an app-menu entry with the icon and needs no sudo. It uses the bundled binary (built on Ubuntu 24.04) and falls back to `cargo build --release` if that cannot run on your system. `./uninstall.sh` removes it; your prompts in `~/.wordesign` are never touched. For all users: `PREFIX=/usr/local sudo -E ./install.sh`.

### New in 1.1

- **Deep blue is the default theme**, plus a new **Gray** (Settings > Appearance). Violet stays as a preset. Anyone still on the old default moves to blue.
- Renamed to **wordesign**, with the logo as window icon and in the menu bar.

### New in 1.0

- **Cost in the preview** - pick a model and press *Calculate cost* to price the exported prompt (input tokens). OpenAI models use the exact tokenizer (tiktoken); Claude and Gemini counts are estimates. Prices come from the LiteLLM price table (provider pricing pages are rendered by JavaScript), refreshed in the background at each launch, cached in `~/.wordesign/.prices.json`, with a bundled snapshot as an offline fallback.
- **Main** replaces *Task* (exports now use `# Main`; old files still open).
- **Zoom is remembered** between launches.
- **Reusable contexts** - File > New > Reusable context saves one; `+ Context` offers empty or a saved one (copied in).

## Layout

- `src/model.rs` — data model, `.aiprompt` file format, markdown compile/export, tests
- `src/store.rs` — `~/.wordesign` storage, naming, atomic writes, tests
- `src/settings.rs` — persistent settings (`~/.wordesign/.settings`)
- `src/linemove.rs` — Alt+Up/Down line moving
- `src/theme.rs` — colors, neon borders, shared button helpers
- `src/app.rs` — the UI

## Credits

- Handwriting font: Patrick Hand (SIL OFL). Spelling dictionary: en_US from SCOWL via wooorm/dictionaries (see `assets/dict/en_US.license`).

## Known limitations

- Selecting text that spans a direction change highlights its whole visual extent in one block (gedit draws separate blocks).
- Diagram labels mixing scripts use a single system font for the whole label.
- Spell checking is English only.
- Find & replace selects one match at a time; it does not highlight all matches at once.
- Folders cannot yet be renamed from the UI (deleting works: right-click).
- `src/fonts.rs`, `src/diagram.rs`, `src/preview.rs` — fonts, diagram rendering, preview splitting.
# wordesign
