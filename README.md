# otail - Oxidized Tail

A TUI-based two-pane log file viewer with search. Written in Rust.

Note: this is an early stage project.

## Features

- Show a log file in the top pane.
- The lower pane shows a filtered view of the view.
- Sync the top pane to the currently selected filter pane line.
- Both panes can tail the file.
- Handles file truncation.

## Future features

(No promises!)

- Coloured matching. Highlight interesting content wherever it appears in the UI.
- Layered filtering. Consecutively apply filters to narrow down a search.

## Installing

Install from a locally clones git repo:

- `cargo install --path .`

or directly:

- `cargo install --git https://github.com/FrankTaylorLieder/otail.git`

## Running

Running:

- `otail <file>`

Note: `otail` only works against files on disk. It does not read from `STDIN`.

## Operation

The TUI opens with two panels: the top one displays the full log file, the
lower one displays a filtered view (empty to start with as no filter is
specified). You switch focus between the panels (`TAB`) and move around the
contents. Key bindings (below) are reminiscent of VIM.

The file contents are displayed without wrapping, one file line per screen
line. You need to scroll left/right to see content off the screen.

To change the filter expression press `/` which opens up a dialogue box to add
or change the filter expression (a regex). When applied, any line that matches
the expression is shown in the filter pane.

When focussed on the filter pane, pressing `s` will sync the current line of
the top pane to the line highlighed in the filter pane. Pressing `S` will
toggle auto-sync, meaning whenever the current line of the filter pane changes,
the content pane will be sycned.

Finally, either pane can be toggled to tailing mode where the content
automatically scrolls to any new content added to the file (similarly any new
filtered content for the filter pane).

### Key bindings

- Movement (applies to the current pane)
  - `h`, `j`, `k`, `l`, `UP`, `DOWN`, `LEFT`, `RIGHT`
    - Move up/down/left/right by a single line or character.
    - Note: the content will only pan left/right if there is content truncated
    off the screen.
  - `u`, `d`
    - Move up/down by 20 lines.
  - `BACKSPACE`, `SPACE`, `PgUp`, `PgDown`
    - Move up and down by a full screen.
  - `H`, `L`
    - Move left/right by 20 characters.
  - `$`, `^`
    - Move to the end and start of lines.
  - `g`, `G`
    - Move to the first/last line of the file.

- Pane
  - `TAB`
    - Toggle the current pane.
  - `+`/`-` (also `=`/`_`)
    - Grow or shrink the current pane height.

- Controls
  - `t`
    - Toggle tailing for the current pane.
  - `s`
    - Sync the content pane with the filter pane.
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

