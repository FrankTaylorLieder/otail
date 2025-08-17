# Development Log

## 2025-06-27 - Fix Case-Insensitive Filtering

**Request**: Case insensitive filtering was not working properly despite being implemented.

**Problem Analysis**: 
- Case-insensitive filtering was already implemented and set as default (`FilterType::SimpleCaseInsensitive`)
- The issue was in `src/common.rs:54` where only the line was converted to `CaseInsensitiveString` but the pattern remained as a regular string
- This caused the comparison to fail because `CaseInsensitiveString::contains()` expected both arguments to be case-insensitive

**Solution**: 
- Fixed `FilterSpec::matches()` method in `src/common.rs:54`
- Initial attempt with `CaseInsensitiveString` failed to compile due to API incompatibility
- Final solution: Changed from: `CaseInsensitiveString::new(line).contains(&self.filter_pattern)`
- Changed to: `line.to_lowercase().contains(&self.filter_pattern.to_lowercase())`
- Removed unused `case_insensitive_string::CaseInsensitiveString` import

**Files Modified**:
- `src/common.rs` - Fixed case-insensitive string matching logic and removed unused import
- `Cargo.toml` - Removed unused `case_insensitive_string` dependency

**Testing Recommendation**: Test with mixed-case filter patterns (e.g., "ERROR") against mixed-case log content (e.g., "error", "Error", "ERROR") to verify case-insensitive matching now works correctly.

## 2025-07-22 - Enhanced Colouring Dialogue with Interactive UI Components

**Request**: Add UI elements to the colouring dialogue box to allow editing of ColouringSpec with a selectable list of rules, pattern editing using existing draw_filter_edit function, and colour picker for foreground/background colours. Tab key should switch focus between sections.

**Implementation Plan**:
1. Expand ColouringEditState structure with fields for rule selection, focus management, and temporary editing state
2. Implement two-pane UI layout: rules list (top) and rule editor (bottom) 
3. Reuse existing draw_filter_edit function for pattern editing (placed on top)
4. Add colour picker for foreground/background selection (placed below pattern editor)
5. Implement tab-based focus management between sections
6. Enhance key event handling for all interactions

**Changes Made**:

### Core Data Structures:
- **Enhanced `ColouringEditState`** (src/tui.rs:201-208):
  - Added `selected_rule_index: usize` for rule selection
  - Added `focus_area: ColouringFocusArea` for tracking current focus
  - Added `filter_edit_state: FilterEditState` to integrate with existing pattern editor
  - Added `selected_fg_color` and `selected_bg_color` for colour selection state

- **Added `ColouringFocusArea` enum** (src/tui.rs:210-215):
  - `RulesList`, `PatternEditor`, `ColourPicker` variants for focus management

- **Enhanced `ColouringSpec`** (src/colour_spec.rs:45-47):
  - Added `rules()` getter method for accessing private rules field

### UI Implementation:
- **Redesigned `draw_colouring_dlg`** (src/tui.rs:964-981):
  - Expanded popup size to 80% × 70% 
  - Split into two equal vertical sections for rules list and editor
  - Added comprehensive help text in title bar

- **Implemented `draw_colouring_rules_list`** (src/tui.rs:983-1022):
  - Scrollable, selectable list showing enabled status, pattern, and colours
  - Visual highlight for selected rule with blue background
  - Focus-aware border styling (thick when focused)
  - Graceful handling of empty rules list

- **Implemented `draw_colouring_edit_section`** (src/tui.rs:1024-1048):
  - Vertical layout: pattern editor (top) and colour picker (bottom)
  - Integration with existing `draw_filter_edit` function for pattern editing
  - Focus-aware border styling for pattern editor section

- **Implemented `draw_colour_picker`** (src/tui.rs:1050-1289):
  - Two-column layout for foreground and background colors
  - Complete color palette: None, Black, Red, Green, Blue, Yellow, Magenta, Cyan, White, Gray
  - Keyboard shortcuts displayed: numbers 0-9 for foreground, Shift+0-9 for background
  - Radio button UI elements for selection visualization

### Event Handling & Focus Management:
- **Comprehensive key event handling** (src/tui.rs:555-606):
  - Tab key cycles through focus areas: RulesList → PatternEditor → ColourPicker
  - Up/Down arrows navigate rules list and update editor
  - Enter applies changes and closes dialog
  - Esc cancels and closes dialog
  - Pattern editor keys (Ctrl+T toggle, Ctrl+S/C/R for filter types)
  - Color selection keys (0-9 for foreground, Shift+0-9 for background)

- **Helper methods for functionality**:
  - `cycle_colouring_focus()`: Tab key focus switching
  - `handle_colouring_up_key()` & `handle_colouring_down_key()`: Navigation
  - `handle_colouring_color_key()`: Color selection with modifier support
  - `load_selected_rule_into_editor()`: Syncs rule to editor when selection changes

### Architecture Integration:
- **Updated `start_edit_colouring`** (src/tui.rs:848-872):
  - Initializes all new state fields properly
  - Sets sensible defaults from existing rules
  - Handles empty rules list gracefully

### Key Features Implemented:
✅ **Selectable, scrollable rules list** with visual indicators
✅ **Reused existing draw_filter_edit function** for pattern editing  
✅ **Comprehensive colour picker** with full palette and keyboard shortcuts
✅ **Tab-based focus management** with visual focus indicators
✅ **Complete key event handling** for all interactions
✅ **Vertical layout** with pattern editor on top, colour picker below
✅ **Integration with existing architecture** and state management

**Files Modified**:
- `src/tui.rs` - Enhanced ColouringEditState, UI implementation, event handling
- `src/colour_spec.rs` - Added rules() getter method, removed unused import

**Testing Recommendation**: Open colouring dialogue with 'C' key, test navigation with Tab, Up/Down arrows, pattern editing with existing controls, and colour selection with number keys. Verify focus indicators and rule list updates work correctly.

## 2025-07-22 - Added Rule Management Features to Colouring Dialogue  

**Request**: Add keys to add new rules with default values, delete existing rules with confirmation, and move rules up/down in the list using Shift+Up/Down.

**Implementation Overview**:
Added comprehensive rule management capabilities with confirmation dialogs, default rule creation, and intuitive keyboard shortcuts for all operations.

**New Features Implemented**:

### Rule Addition:
- **Keys**: `Insert` or `+` to add new rule
- **Behavior**: Creates rule with default values (enabled, empty pattern, no colors)
- **Position**: Inserts after currently selected rule
- **Auto-selection**: Automatically selects newly created rule and loads into editor

### Rule Deletion with Confirmation:
- **Keys**: `Delete` or `-` to initiate deletion
- **Confirmation Flow**: 
  1. Displays warning message: "⚠️ Press 'y' to DELETE rule, any other key to CANCEL"
  2. `y` key confirms deletion
  3. Any other key cancels deletion
- **Safety**: Prevents accidental deletions, handles empty list gracefully
- **Auto-adjustment**: Updates selection after deletion, loads remaining rule into editor

### Rule Reordering:
- **Keys**: `Shift+Up` and `Shift+Down` to move rules
- **Behavior**: Swaps rule positions in the list
- **Selection tracking**: Keeps focus on the moved rule
- **Boundaries**: Handles first/last positions gracefully

**Technical Implementation**:

### Enhanced Data Structures:
- **ColouringEditState**: Added `pending_deletion: Option<usize>` field for deletion confirmation state
- **ColouringRule**: Added `default()` constructor for creating rules with sensible defaults
- **ColouringSpec**: Added mutable methods:
  - `add_rule(rule, index)` - Insert rule at position
  - `remove_rule(index)` - Delete rule and return it
  - `move_rule_up(index)` / `move_rule_down(index)` - Swap with adjacent rules  
  - `update_rule(index, rule)` - Replace rule in-place

### Key Event Handling:
- **Priority-based matching**: Shift+Up/Down handled before regular Up/Down
- **State-aware responses**: Different behavior when deletion is pending
- **Confirmation flow**: `y` confirms deletion, any other key cancels

### Helper Methods Added:
- `handle_colouring_add_rule()` - Creates and inserts new rule
- `handle_colouring_delete_rule()` - Initiates deletion confirmation
- `handle_colouring_confirm_deletion()` - Completes deletion process
- `handle_colouring_cancel_deletion()` - Cancels pending deletion
- `handle_colouring_move_rule_up()` / `handle_colouring_move_rule_down()` - Rule reordering
- `update_selected_rule_from_editor()` - Saves editor changes to selected rule
- `apply_colouring_changes()` - Commits all changes to main colouring spec

### UI Enhancements:
- **Dynamic title**: Shows deletion confirmation message when deletion is pending
- **Updated help text**: Comprehensive key binding reference in title bar
- **Smart defaults**: New rules start with enabled=true, empty pattern, no colors

**Key Bindings Summary**:
- **Tab**: Switch focus between sections
- **↑/↓**: Navigate rules list
- **Shift+↑/↓**: Move current rule up/down in list  
- **Insert/+**: Add new rule with defaults
- **Delete/-**: Prompt for rule deletion
- **y**: Confirm deletion (when prompted)
- **Any other key**: Cancel deletion (when prompted)
- **0-9**: Select foreground color
- **Shift+0-9**: Select background color
- **Ctrl+T/S/C/R**: Pattern editor controls
- **Enter**: Apply all changes and close
- **Esc**: Cancel and close

**Files Modified**:
- `src/tui.rs` - Rule management event handling, helper methods, UI updates
- `src/colour_spec.rs` - Mutable methods for rule manipulation, default rule constructor

**Testing Recommendation**: Test rule addition with Insert/+, deletion with Delete/- (confirm with 'y', cancel with other keys), reordering with Shift+Up/Down, and ensure all operations update the UI correctly. Verify deletion confirmation prevents accidental loss of rules.

## 2025-07-23 - Updated README with Recently Added Key Bindings

**Request**: Add the recently added key bindings to the README, particularly the colouring functionality that was missing from the documentation.

**Problem Analysis**: 
- The README.md contained comprehensive key bindings documentation but was missing recently implemented colouring dialog functionality
- The colouring dialog (`C` key) was implemented but not documented in the user-facing README
- All colouring dialog key bindings were missing, making the feature difficult for users to discover and use

**Solution**:
- Added `C` key binding to the main Controls section to open colouring edit dialogue
- Added complete "Colouring dialogue" section after the existing "Filter dialogue" section
- Documented all colouring dialog key bindings including:
  - Focus management (Tab/Shift+Tab)
  - Rules list navigation (j/k/UP/DOWN) 
  - Rule management (Insert/+, Delete/-, y for confirmation)
  - Rule reordering (Shift+UP/DOWN, Shift+K/J)
  - Pattern editor controls (Ctrl+t/s/c/r)
  - Color selection (0-9 for foreground, Shift+0-9 for background)
  - Dialog controls (Enter/Esc)

**Files Modified**:
- `README.md` - Added colouring dialog key binding to Controls section and comprehensive Colouring dialogue section with all key bindings

**Testing Recommendation**: Verify the README now accurately reflects all available key bindings in the current version of otail, particularly testing that users can discover and use the colouring functionality through the documentation.

## 2025-07-23 - Restricted Color Selection to Colour Picker Focus Only

**Request**: Update keys so that colour selection is only possible when the colours pane is selected, not from any focus area within the colouring dialog.

**Problem Analysis**: 
- The current implementation allowed color keys (0-9 and Shift+0-9) to work from any focus area within the colouring dialog
- This could lead to accidental color changes when users were focused on the Rules List or Pattern Editor
- The README documentation was updated to show focus requirement, but the code didn't match this behavior

**Solution**:
- **Code Changes**: Moved color key handling from global scope (lines 623-632) into the focus-specific `ColouringFocusArea::ColourPicker` section
- **Behavior Change**: Color selection keys now only work when the colour picker pane is specifically focused
- **Documentation Update**: Enhanced README color selection section with complete color mapping details

**Implementation Details**:
- Removed global color key handling that worked regardless of focus area
- Added color key matching within the `ColouringFocusArea::ColourPicker` match arm
- Preserved all existing color key mappings (0-9 for foreground, Shift+0-9 for background)
- Maintained immediate rule updates via `update_selected_rule_from_editor()`

**Files Modified**:
- `src/tui.rs` - Moved color key handling to focus-specific section (lines 624-634)
- `README.md` - Enhanced color selection documentation with complete color mappings

**Testing Recommendation**: Test that color keys (0-9, Shift+0-9) only work when the colour picker pane is focused, and are ignored when focus is on Rules List or Pattern Editor. Verify Tab navigation still works correctly and color changes are applied immediately when the colour picker is focused.

## 2025-07-23 - Compact Multi-Column Color Picker Layout

**Request**: Change the layout of the colouring panes to use less vertical space by listing several colours per line, adjusting based on the size of the pane.

**Problem Analysis**:
- The original color picker layout used two separate columns (Foreground/Background) with 10 lines each plus borders
- This required approximately 12-13 lines total vertical space, which was quite tall
- The layout was not responsive to terminal width and could not adapt to smaller screens
- Each color had verbose labels like "[Shift+1] (None)" taking up horizontal space

**Solution**:
- **Redesigned to compact multi-column layout**: Colors now display in a grid format that adapts to available width
- **Dynamic column calculation**: Number of columns adjusts based on terminal width (1-5 columns, minimum 16 characters per entry)
- **Unified foreground/background display**: Each color shows both fg/bg selection status in one entry: "● ○ 1:None"
- **Full color names**: Clear, readable color names (e.g., "Black", "Red", "Green", "Magenta")
- **Reduced vertical space**: New layout needs only 4 lines minimum (down from 6) and typically uses 3-4 lines total

**Implementation Details**:

### New Layout Algorithm:
- **Width-based columns**: `num_cols = max(1, min(5, available_width / 16))`
- **Row calculation**: `num_rows = (10 colors + num_cols - 1) / num_cols`
- **Grid arrangement**: Colors arranged in row-major order across columns

### Visual Format:
- **Old format**: Two separate columns with "[1] (None)" and "[Shift+1] (None)"
- **New format**: "● ○ 1:None" (or "● ○ 7:Magenta") where:
  - Left ● = foreground selected, ○ = unselected
  - Right ● = background selected, ○ = unselected  
  - "1:None" = key and color name

### Space Efficiency:
- **Minimum height reduced**: From `Constraint::Min(6)` to `Constraint::Min(4)`
- **Typical usage**: 3-4 lines for colors + 1 help line = 4-5 lines total (vs. 12-13 previously)
- **Responsive width**: Automatically uses 1-5 columns based on available space

### Enhanced User Experience:
- **Updated title**: "Colours (0-9=fg, Shift+0-9=bg)" provides immediate key reference
- **Inline help**: "● = selected, ○ = unselected (left=fg, right=bg)" when space allows
- **Preserved functionality**: All color selection behavior remains identical

**Files Modified**:
- `src/tui.rs` - Completely rewrote `draw_colour_picker` function with dynamic multi-column layout, reduced minimum height constraint from 6 to 4 lines

**Testing Recommendation**: Test the color picker dialog with various terminal widths to verify the dynamic column layout works correctly. Verify that all 10 colors display properly in narrow and wide terminals, color selection still works with 0-9 and Shift+0-9 keys, and the visual indicators (● ○) correctly show foreground/background selection status.

## 2025-07-23 - Updated Color Picker to Use Full Color Names

**Request**: Use full colour names instead of abbreviated ones in the color picker.

**Changes Made**:
- **Updated color names**: Changed from abbreviated names ("Blk", "Grn", "Blu", etc.) to full names ("Black", "Green", "Blue", "Magenta", etc.)
- **Adjusted layout calculation**: Increased minimum entry width from 12 to 16 characters to accommodate longer color names like "Magenta"
- **Maintained responsiveness**: Dynamic column calculation still works with the new width requirements

**Files Modified**:
- `src/tui.rs` - Updated color data array with full color names, adjusted `min_entry_width` from 12 to 16 characters

**Testing Recommendation**: Verify that full color names display correctly in the color picker and that the dynamic layout still adapts properly to different terminal widths with the updated spacing requirements.

## 2025-07-23 - Added Column Alignment to Color Picker

**Request**: Arrange the color picker items in aligned columns for better visual organization.

**Changes Made**:
- **Added column width calculation**: `col_width = available_width / num_cols` to determine uniform column spacing
- **Implemented left-aligned padding**: Each entry (except the last column) is padded to the calculated column width using `format!("{:<width$}", entry, width = col_width)`
- **Improved visual organization**: Colors now display in properly aligned columns regardless of color name length

**Technical Details**:
- **Alignment method**: Left-aligned padding ensures consistent column spacing
- **Edge case handling**: Last column doesn't get padding to avoid unnecessary trailing spaces
- **Dynamic adaptation**: Column alignment works with the existing responsive layout system

**Files Modified**:
- `src/tui.rs` - Added column width calculation and left-aligned padding for color entries

**Testing Recommendation**: Test the color picker with various terminal widths to verify that colors align properly in columns and that the alignment works correctly with 1-5 columns depending on available space.

## 2025-07-23 - Changed Color Keys to Match First Letter of Color Names

**Request**: Change the keys for colours to match the first letter of the colour for the foreground and shifted version for background. Grey should use 'x' as its key.

**Changes Made**:
- **Updated color key mappings**: Changed from number keys (0-9) to letter keys based on color names:
  - `n`/`N` = None (no color)
  - `b`/`B` = Black
  - `r`/`R` = Red  
  - `g`/`G` = Green
  - `u`/`U` = Blue (using 'u' since 'b' is taken by Black)
  - `y`/`Y` = Yellow
  - `m`/`M` = Magenta
  - `c`/`C` = Cyan
  - `w`/`W` = White
  - `x`/`X` = Gray (as requested, using 'x' instead of 'g')

- **Updated key handling**: Modified `handle_colouring_color_key()` function to recognize new letter-based key mappings
- **Updated event matching**: Changed key matching pattern from numbers/symbols to letters (both lowercase and uppercase)
- **Updated display**: Color picker now shows letter keys (e.g., "n:None", "b:Black") instead of numbers
- **Updated documentation**: README now reflects new letter-based key bindings

**Technical Details**:
- **Foreground colors**: Lowercase letters (`n`, `b`, `r`, `g`, `u`, `y`, `m`, `c`, `w`, `x`)
- **Background colors**: Uppercase letters (`N`, `B`, `R`, `G`, `U`, `Y`, `M`, `C`, `W`, `X`)
- **Special handling**: Blue uses 'u' to avoid conflict with Black's 'b', Gray uses 'x' as requested
- **Maintained functionality**: All existing color selection behavior preserved with new key mappings

**Files Modified**:
- `src/tui.rs` - Updated color data array, key handling function, and event matching patterns
- `README.md` - Updated color selection key bindings documentation

**Testing Recommendation**: Test all new letter-based color keys to verify they correctly set foreground (lowercase) and background (uppercase) colors. Verify that 'u' selects Blue, 'x' selects Gray, and all other letters match their respective color names.

## 2025-07-23 - Removed Help Text from Color Picker Pane

**Request**: Remove the selected/unselected help text from the colour pane.

**Changes Made**:
- **Removed help text**: Eliminated the bottom help line "● = selected, ○ = unselected (left=fg, right=bg)" from the color picker
- **Cleaner interface**: Color picker now shows only the color options without additional explanatory text
- **More space efficient**: Removes the extra lines used for help text, making the interface more compact

**Files Modified**:
- `src/tui.rs` - Removed the conditional help text addition in the `draw_colour_picker` function

**Testing Recommendation**: Verify that the color picker displays cleanly without the help text and that the visual indicators (● ○) are still intuitive without the explanation.

## 2025-07-23 - Added Scrollable Rules List with Scrollbar Indicator

**Request**: Make the top rules list scrollable with a scrollbar indication, like the content pane.

**Changes Made**:
- **Added scrollbar state management**: Added `rules_scroll_state: ScrollbarState` and `rules_list_state: ListState` to `ColouringEditState`
- **Converted to List widget**: Changed from `Paragraph` to `List` widget with proper selection highlighting and scrolling support
- **Added scrollbar indicator**: Implemented vertical scrollbar on the right side of the rules list, matching the content pane style
- **Updated navigation functions**: All rule navigation (up/down, add, delete, move) now properly updates both selection and scrollbar position
- **Proper state synchronization**: List selection and scrollbar position stay in sync during all operations

**Technical Implementation**:

### New State Management:
- **ScrollbarState**: Tracks scrollbar position and content length for proper scrollbar sizing
- **ListState**: Manages list selection and handles scrolling behavior
- **Synchronized updates**: All rule operations update both states to maintain consistency

### Widget Changes:
- **List widget**: Replaced `Paragraph` with `List` for native scrolling support
- **ListItem creation**: Rules now rendered as `ListItem` objects with proper formatting
- **Highlight styling**: Selected rule shows with bold text and "> " highlight symbol
- **Scrollbar rendering**: Added `Scrollbar` widget with vertical right orientation matching content pane

### Navigation Updates:
- **Up/Down keys**: Update both `selected_rule_index` and scrollbar position
- **Rule management**: Add, delete, and move operations maintain scrollbar sync  
- **Selection bounds**: Proper handling of empty lists and boundary conditions

**Files Modified**:
- `src/tui.rs` - Added scrollbar/list state fields, updated `draw_colouring_rules_list` to use List widget with scrollbar, updated all navigation functions to sync scrollbar position

**Testing Recommendation**: Test the rules list with many rules to verify scrolling works correctly. Navigate with j/k keys, add/delete rules, and move rules up/down to ensure the scrollbar indicator properly reflects the current position and list length. Verify that the scrollbar appears/disappears appropriately based on content length vs. visible area.

## 2025-07-23 - Fixed Rule Summary Update on Pattern Property Changes

**Request**: When toggling enabled on a filter rule pattern, update the rule summary immediately.

**Problem Analysis**: 
- When using Ctrl+T to toggle the enabled state of a pattern in the colouring dialog, the rule summary in the rules list didn't update immediately
- The same issue existed for filter type changes (Ctrl+S, Ctrl+C, Ctrl+R) 
- Users had to navigate away and back to see the updated rule summary
- Text input changes were already working correctly

**Solution**:
- **Added immediate updates**: Added `self.update_selected_rule_from_editor()` calls to all pattern property change handlers
- **Enabled toggle**: Ctrl+T now immediately updates the rule summary with ✓/✗ indicator
- **Filter type changes**: Ctrl+S, Ctrl+C, Ctrl+R now immediately update the rule summary
- **Maintained existing behavior**: Text input changes already had proper updating (line 625)

**Changes Made**:
- **Ctrl+T (toggle enabled)**: Added `update_selected_rule_from_editor()` call after toggling enabled state
- **Ctrl+S (case insensitive)**: Added `update_selected_rule_from_editor()` call after filter type change
- **Ctrl+C (case sensitive)**: Added `update_selected_rule_from_editor()` call after filter type change  
- **Ctrl+R (regex)**: Added `update_selected_rule_from_editor()` call after filter type change

**Files Modified**:
- `src/tui.rs` - Added `update_selected_rule_from_editor()` calls to pattern property change handlers (lines 598, 604, 610, 616)

**Testing Recommendation**: Test all pattern editing shortcuts (Ctrl+T, Ctrl+S, Ctrl+C, Ctrl+R) in the colouring dialog to verify that the rule summary in the rules list updates immediately. Verify that the enabled state (✓/✗) and filter type are reflected in real-time without needing to navigate away and back.

## 2025-07-23 - Added Rule Toggle from Rules Pane

**Request**: Allow toggling a rule from the rules pane as well as from the pattern pane.

**Changes Made**:
- **Added rules pane toggle**: Users can now press `t` in the rules list to toggle the enabled/disabled state of the currently selected rule
- **Dual toggle support**: Rules can now be toggled from both:
  - Rules List (when focused): `t` key
  - Pattern Editor (when focused): `Ctrl+t` key
- **Immediate synchronization**: When toggling from rules pane, the pattern editor state is immediately updated to reflect the change
- **Updated UI guidance**: Rules list title now includes "t=toggle" in the help text

**Technical Implementation**:
- **Focus-specific handling**: Added key handling for `ColouringFocusArea::RulesList` to process the `t` key
- **Rule update mechanism**: Uses `ColouringSpec::update_rule()` method to modify the rule in place
- **State synchronization**: Updates both the rule in the spec and the editor state (`filter_edit_state.enabled`) to maintain consistency
- **No conflicts**: Uses plain `t` key for rules pane vs `Ctrl+t` for pattern editor to avoid key conflicts

**Key Bindings**:
- **Rules List (focused)**: `t` - Toggle enabled/disabled state of current rule
- **Pattern Editor (focused)**: `Ctrl+t` - Toggle pattern enabled/disabled (existing)

**Files Modified**:
- `src/tui.rs` - Added rules list key handling for toggle functionality, updated rules list title with toggle help text
- `README.md` - Added documentation for new `t` key binding in Rules List section

**Testing Recommendation**: Test toggling rules from both the rules list (`t` key) and pattern editor (`Ctrl+t` key). Verify that the enabled state (✓/✗) updates immediately in the rules list when toggling from either location, and that the pattern editor enabled checkbox stays synchronized when toggling from the rules list.

## 2025-07-23 - Added Rule Numbering to Rules Pane

**Request**: Number the rules in the rules pane.

**Changes Made**:
- **Added rule numbering**: Each rule in the rules list now displays with a sequential number (1, 2, 3, etc.)
- **Clear visual organization**: Numbers help users quickly identify and reference specific rules
- **Consistent formatting**: Rule display format is now: `{number}. {enabled} {pattern} → fg:{color}/bg:{color}`

**Technical Implementation**:
- **Added enumerate()**: Used `enumerate()` iterator to get index along with rule data
- **Updated text formatting**: Modified format string from `"{} {} → fg:{}/bg:{}"` to `"{}. {} {} → fg:{}/bg:{}"` 
- **1-based numbering**: Used `index + 1` to display human-friendly numbering starting from 1

**Example Display**:
```
1. ✓ error → fg:Red/bg:None
2. ✗ warning → fg:Yellow/bg:None  
3. ✓ info → fg:Blue/bg:None
```

**Files Modified**:
- `src/tui.rs` - Added enumerate() and updated rule text formatting in `draw_colouring_rules_list` function

**Testing Recommendation**: Verify that rules display with sequential numbering (1, 2, 3, etc.) in the rules list. Test with multiple rules to ensure numbering remains consistent when adding, deleting, or reordering rules.

## 2025-08-17 - Added Custom Config File Command Line Option

**Request**: Add a command line argument (--config and -c) to load a specific config file rather than searching for them. Ensure the README is updated with this new feature.

**Implementation Overview**:
Added ability to specify a custom config file path via command line arguments, with strict validation that exits if the specified file doesn't exist.

**Changes Made**:

### Command Line Argument:
- **Added --config/-c option**: New optional argument to specify custom config file path
- **Help text**: "Specify a custom config file path"
- **Error handling**: Application exits with error message if specified config file doesn't exist

### Config Loading Enhancement:
- **New function**: `load_config_from(config_path: Option<String>) -> Result<LocatedConfig>`
- **Strict validation**: Returns error if specified config file doesn't exist
- **Fallback behavior**: Falls back to default search when no config path specified
- **Backward compatibility**: Existing `load_config()` function maintained for other uses

### Integration:
- **Main function**: Updated to load config early and exit on error before other initialization
- **Tui constructor**: Modified to accept `LocatedConfig` parameter instead of loading internally
- **Error flow**: Clear error messages displayed to user before application exit

### Documentation:
- **README updates**: Added usage examples with new --config option
- **Config section**: Enhanced with information about custom config file behavior
- **Error behavior**: Documented that otail exits if specified config file doesn't exist

**Technical Details**:

### Error Handling Flow:
1. Parse command line arguments
2. Attempt to load config file (custom or default search)  
3. If custom config file specified but doesn't exist: error and exit
4. If no custom config specified: use existing search behavior
5. Continue with normal initialization only if config loading succeeds

### API Changes:
- **main.rs**: Added config loading with error handling
- **config.rs**: Added `load_config_from()` function with Result return type
- **tui.rs**: Modified `Tui::new()` to accept config parameter
- **README.md**: Added usage examples and config documentation

**Files Modified**:
- `src/main.rs` - Added config import, early config loading with error handling, pass config to Tui::new()
- `src/config.rs` - Added load_config_from() function with custom path support and error handling
- `src/tui.rs` - Modified Tui::new() to accept LocatedConfig parameter instead of loading internally
- `README.md` - Added --config option to usage section and enhanced config documentation
- `DEVELOPMENT.md` - Added this development log entry

**Key Benefits**:
- **Explicit config control**: Users can specify exact config file location
- **Clear error handling**: No silent failures when config file doesn't exist
- **Backward compatibility**: Existing behavior preserved when no --config specified
- **Developer workflow**: Easier testing with different config files

**Testing Recommendation**: Test --config with existing files, non-existent files (should exit with error), and without --config (should use default search). Verify help output shows new option and all existing functionality remains unchanged.