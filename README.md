# otail - Oxidized Tail

A TUI-based two-pane log file viewer with search. Written in Rust.

Note: this is an early stage project.

## Features

- Shows a log file in the top content pane.
- The lower filtered pane shows a filtered view of the file.
- Highlight interesting lines by colouring them based on their content in
either pane.
- Sync the top pane to the currently selected filtered pane line.
- Both panes can tail the file.
- Handles file truncation.
- Load and save configuration changes, to either a project local directory or
home directory.

## Future features

(No promises!)

- Layered filtering. Consecutively apply filters to narrow down a search.

## Installing

Install from a locally cloned git repo:

- `cargo install --path .`

or directly from git:

- `cargo install --git https://github.com/FrankTaylorLieder/otail.git`

## Running

Run:

- `otail <file>`
- `otail --config <config-file> <file>` or `otail -c <config-file> <file>`

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
or change the matching pattern for the filter. Patterns can be simple text
matches (case sensitive or insensitive), or regular expressions. When applied,
any line that matches the expression is shown in the filter pane.

Pressing `s` will sync the content pane to match the current line in the
filtered pane. Pressing `S` will toggle auto-sync, meaning whenever the current
line of the filtered pane changes, the content pane will be synced to match.

You can highlight content across either pane by opening the colouring dialogue
by pressing `C`. In this dialogue you can create a set of ordered colouring
rules, which are applied to all output. The first rule that matches defines the
colour of a line. Each rule as a matching pattern (just the same as filtering
above) Each rule as a matching pattern (just the same as filtering above). You
can set foreground and/or background colours for each rule. (Currently
colouring rules are lost between `otail` sessions. A default error rule is
provided for each session.)

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
  - `C`
    - Open the colouring edit dialogue.
  - `q`
    - Quit `otail`.

- Filter dialogue
  - `Esc`
    - Close the dialogue.
  - `Enter`
    - Apply the current filter.
  - `t`
    - Toggle the filter enabled.

- Colouring dialogue
  - (`Shift+`)`Tab`
    - Cycle focus forward/backwards: Rules List → Pattern Editor → Colour Picker → Rules List.
  - `Esc`
    - Cancel changes and close the dialogue.
  - `Enter`
    - Apply all colouring changes and close the dialogue.
  - Rules List (when focused)
    - `j`, `k`, `DOWN`, `UP`
      - Navigate up/down in the rules list.
    - `t`
      - Toggle enabled/disabled state of current rule.
    - `Insert`, `+`
      - Add new rule with default values.
    - `Delete`, `-`
      - Prompt to delete current rule.
    - `Shift+UP`/`DOWN`, `Shift+K`/`J`
      - Move current rule up/down in list.
  - Pattern Editor (when focused)
    - `Ctrl+t`
      - Toggle pattern enabled/disabled.
    - `Ctrl+s`
      - Set pattern type to Simple Case Insensitive.
    - `Ctrl+c`
      - Set pattern type to Simple Case Sensitive.
    - `Ctrl+r`
      - Set pattern type to Regex.
  - Colour Selection (when focused)
    - Letters: `n` (None), `b` (Black), `r` (Red), `g` (Green), `u` (Blue), `y` (Yellow), `m` (Magenta), `c` (Cyan), `w` (White), `x` (Gray) for foreground colours.
    - `Shift+letters`: `N`, `B`, `R`, `G`, `U`, `Y`, `M`, `C`, `W`, `X` for background colours.

## Config

The colouring rules can be persisted between uses of `otail`. By default
colouring configuration is loaded from the first of the following locations:

- `./otail.yaml`
- `./.otail.yaml`
- `$HOME/.config/otail.yaml`

Alternatively, you can specify a custom config file using the `--config` or `-c` 
option. If the specified config file does not exist, `otail` will exit with an error.

To start using saved configurations simply create an empty config file in your
preferred location. The default rules will be loaded and any changes to the
colouring rules will be saved to this file.

A configuaration file can be made readonly by editing the file and setting
`readonly: true`. You might like to have some default configuration in the your
`$HOME/.config/otail.yaml` which you can copy into projects, changing
`readonly` back to false to enable local changes to persist.

After each change in colouring rules the configuration is saved to the same
location it was loaded from, unless it was marked `readonly`.

If no configuration file is found a default set of colouring rules is used and
changes will not be saved.

## Contributions

- Please contact the author if you are interested in contributing.
- Raise feature requests and bugs in the issue tracker.

## License

This software is published under the MIT License.

