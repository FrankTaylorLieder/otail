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