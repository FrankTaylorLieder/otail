# otail - Oxidized Tail

A TUI-based two-pane log file viewer with search. Written in Rust.

Note: this is an early stage project.

## Features

- Shows a log file in the top content pane.
- The lower filtered pane shows a filtered view of the file.
- Sync the top pane to the currently selected filtered pane line.
- Both panes can tail the file.
- Handles file truncation.

## Future features

(No promises!)

- Colouring. Highlight interesting content wherever it appears in the UI.
- Layered filtering. Consecutively apply filters to narrow down a search.

## Installing

Install from a locally cloned git repo:

- `cargo install --path .`

or directly from git:

- `cargo install --git https://github.com/FrankTaylorLieder/otail.git`

## Running

Run:

- `otail <file>`

Note: `otail` only works against files on disk. It does not read from `STDIN`.

You can enable logging:

- `RUST_LOG=trace otail <file>`
  - Logging levels: `trace`, `debug`, `info`, `warn`, `error`

## Operation

The TUI opens with two panes: the top one displays the full log file (content
pane), the lower one displays a filtered view (filtered pane, empty to start
with as no filter is specified). You can switch focus between the panes
(`TAB`) and move around the contents. Key bindings (below) are reminiscent of
VIM.

The file content is displayed without wrapping, one file line per screen line.
You need to scroll left/right to see content off the screen.

To change the filter expression press `/` which opens up a dialogue box to add
or change the filter expression (a regex). When applied, any line that matches
the expression is shown in the filter pane.

Pressing `s` will sync the current line of the content pane to the line current
line in the filtered pane. Pressing `S` will toggle auto-sync, meaning whenever
the current line of the filtered pane changes, the content pane will be synced.

Finally, either pane can be toggled to tailing mode which automatically scrolls
to any new content. Tailing is cancelled in a pane when manually changing the
current line or when the content is synced with the filter.

### Key bindings

Note: the key bindings may change before this reaches its first stable release.

- Movement (applies to the current pane)
  - `h`, `j`, `k`, `l`, `LEFT`, `DOWN`, `UP`, `RIGHT`
    - Move up/down/left/right by a single line or character.
    - Note: the content will only pan left/right if there is content truncated
    off the screen.
  - `u`, `d`
    - Move up/down by 20 lines.
  - `BACKSPACE`, `SPACE`, `PgUp`, `PgDown`
    - Move up and down by a full screen.
  - `H`, `L`
    - Move left/right by 20 characters.
  - `$`, `0`
    - Move to the end and start of lines.
  - `g`, `G`
    - Move to the first/last line of the file.
  - `z`
    - Center the current line in the window.

- Pane
  - `TAB`
    - Toggle the current pane.
  - `+`/`-` (also `=`/`_`)
    - Grow or shrink the current pane height.

- Controls
  - `t`
    - Toggle tailing for the current pane.
  - `s`
    - Sync the content pane with the filtered pane.
  - `S`
    - Toggle auto-sync.
  - `/`
    - Open the filter edit dialogue.
  - `q`
    - Quit `otail`.

- Filter dialogue
  - `Esc`
    - Close the dialogue.
  - `Enter`
    - Apply the current filter.
  - `t`
    - Toggle the filter enabled.


## Contributions

- Please contact the author if you are interested in contributing.
- Raise feature requests and bugs in the issue tracker.

## License

This software is published under the MIT License.

