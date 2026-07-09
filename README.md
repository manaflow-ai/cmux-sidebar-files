# cmux-sidebar-files

`cmux-sidebar-files` is a narrow, yazi-inspired file explorer for the cmux sidebar. It follows the [cmux sidebar plugin contract](https://github.com/manaflow-ai/cmux-sidebar-fzf#sidebar-plugin-contract): it is an ordinary TUI, reads the cmux JSON-lines socket from `CMUX_TUI_SOCKET` (falling back to `CMUX_MUX_SOCKET`), never exits on `Esc`, and exits cleanly on `Ctrl-C`.

The browser starts at the focused pane's working directory. It discovers the focused PTY surface from `list-workspaces` and reads its cwd with the protocol-6 `process-info` command. If cwd is unavailable (including when the focused tab is a browser or the server is older), it falls back to the plugin process's own cwd.

## Keys

- `Up` / `Down`, `Ctrl-K` / `Ctrl-J`: move the selection
- `Right` / `Enter` on a directory: descend
- `Left` / `h`: go to the parent directory
- `Enter` on a file: run `$EDITOR <absolute-path>` (or `vi`) in a new tab in the focused pane
- `c` on a directory: send `cd '<absolute-path>'` to the focused pane
- `o` on an `.html` or `.md` file: open its `file://` URL in a browser tab
- `.`: toggle dotfiles
- `/`: filter the current directory by a case-insensitive substring
- `Esc`: clear the filter, then leave filter input; it never exits the plugin
- `~`: re-root at the focused pane's cwd and resume following its cwd
- `Ctrl-C`: exit cleanly

Descending or moving to a parent pins the browser at that location. While unpinned, the two-second refresh follows changes to the focused pane's cwd. Press `~` to unpin and re-root.

Opening an editor uses the protocol-6 `run` wire command because `new-tab` only accepts pane/cwd/size and has no command field. `run` is the server's implemented new-PTY-tab command with direct argv execution.

## Standalone Development

Run cmux, find its socket path, and pass it to the plugin:

```sh
CMUX_TUI_SOCKET=/path/to/cmux-tui.sock cargo run
```

Socket paths commonly follow `$TMPDIR/cmux-tui-<uid>/<session>.sock`. Running without either socket environment variable is supported and renders a helpful reconnect screen instead of panicking.

## Install With cmux

```sh
cmux-tui plugin install https://github.com/manaflow-ai/cmux-sidebar-files
```

## Build and Test

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

## Notes

Symlinked directories are listed as files and cannot be descended into; this also guarantees symlink cycles can never cause runaway traversal. `$EDITOR` must be a single executable name or path (it is passed as `argv[0]` without shell interpretation, which is what makes filenames with quotes safe); multi-word values like `code -w` are not supported.
