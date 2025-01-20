# rtail

A TUI-based two-pane log file viewer with search.

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

- `cargo install --git https://github.com/FrankTaylorLieder/rtail.git`

## Running

Running:

- `rtail <file>`

Note: `rtail` only works against files on disk. It does not read from `STDIN`.

## Contributions

- Please contact the author if you are interested in contributing.
- Raise feature requests and bugs in the issue tracker.

## License

This software is published under the MIT License.

