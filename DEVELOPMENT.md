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