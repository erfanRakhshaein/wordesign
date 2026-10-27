Welcome! Everything below is written for you once; delete this prompt whenever you like (right-click it in the sidebar), or reopen it from the Help menu.

## The basics

- Each prompt is one file in `~/.wordesign`. It saves itself as you type.
- Start writing right away. Add structure only when you want it with the toolbar buttons: Context, Definitions, Rules, Code, Flow.
- Export copies a minimal markdown version to the clipboard. Benchmarks never leave the app.

## Shortcuts

- `Ctrl+N` new prompt: type its name, press Enter, start writing
- `Ctrl+Enter` preview on and off. Inside a flow, code block or variable card it collapses that card instead
- `Ctrl+E` export (works inside the preview too)
- `Ctrl+F` find, `Ctrl+H` find and replace. `F3` and `Shift+F3` jump between matches
- `Ctrl+P` open the pad: paste and tidy text, then `Ctrl+Enter` drops it at your cursor
- `Ctrl+K` commit the open prompt to its history (tracked prompts only)
- `Alt+Up` and `Alt+Down` move the current line or selection
- `Alt+V` types a variable tag with the cursor inside
- `Ctrl+Plus` and `Ctrl+Minus` zoom

## Find and replace

- Tick the regex box to use regular expressions. In the replacement, `\n` is a new line, `\t` a tab and `$1` the first group.
- Example: find `,\s*` and replace with `\n` to turn a comma list into one item per line.
- It searches the tab you are on. In the pad it searches the pad.

## Variables

- Write a tag like `{{codechunk_1}}` anywhere in the Context, Main or Rules. It appears in the Variables tab as a card to fill in.
- Variables show in cyan in the preview, click one to jump to its value. Export swaps each tag for its value and refuses to run while a value is missing.
- Tags written as inline code are left alone, which is how this page can show them.

## Folders and temp

- Drag prompts onto folders. Right-click a folder or prompt to delete it for good.
- The `temp` folder is scratch space and empties itself (Settings, General, sets how often).
- Settings, Appearance changes the theme color.

## History

- Tick `track` next to the prompt name to keep its history with git. Not available in `temp`.
- `Ctrl+K` asks for a short note and commits that prompt only.
- Right-click a tracked prompt and choose Rollback to browse and restore earlier versions. Your current state is saved first, so a rollback can always be undone.

## In the preview

- Misspelled words are red, click for suggestions or use Fix all.
- Words you defined in Definitions are cyan, click to read them.
- Pick a model next to Calculate cost to see what sending the prompt would cost. Only the input tokens are counted. OpenAI counts are exact, other providers are estimates. Prices are refreshed at every launch.

## Reusable context

- Choose File, New, Reusable context to save a context you use often.
- When you click `+ Context` you can start empty or copy in one of your saved contexts.

## Zoom

- `Ctrl+Plus` and `Ctrl+Minus` change the scale. Your choice is remembered for the next launch.

Happy prompting.
