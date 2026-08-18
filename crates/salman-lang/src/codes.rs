//! Stable diagnostic codes.
//!
//! Codes appear in users' CI filters and lint suppressions, so once published a
//! code keeps its meaning for ever. Retiring one means never reusing the
//! number.
//!
//! * `E01xx` — lexical
//! * `E02xx` — syntactic
//! * `E03xx` — declarations and symbols
//! * `E04xx` — types
//! * `W0xxx` — warnings
//! * `U0xxx` — constructs salman does not implement yet

use salman_core::diag::DiagCode;

/// A byte that cannot begin any token.
pub const E_UNEXPECTED_CHARACTER: DiagCode = DiagCode("E0101");
/// A block comment that runs to the end of the file.
pub const E_UNTERMINATED_COMMENT: DiagCode = DiagCode("E0102");
/// A string literal that runs to the end of its line or the end of the file.
pub const E_UNTERMINATED_STRING: DiagCode = DiagCode("E0103");
/// A `$` escape that is not one of the eight defined combinations or `$xx`.
pub const E_BAD_ESCAPE: DiagCode = DiagCode("E0104");
/// A based literal whose radix is not 2, 8 or 16.
pub const E_BAD_RADIX: DiagCode = DiagCode("E0105");
/// A digit that does not exist in the literal's radix, such as `2#2`.
pub const E_BAD_DIGIT: DiagCode = DiagCode("E0106");
/// A numeric literal too large for any salman integer type.
pub const E_LITERAL_OUT_OF_RANGE: DiagCode = DiagCode("E0107");
/// A real literal that is not `digits.digits[exponent]`.
pub const E_BAD_REAL: DiagCode = DiagCode("E0108");
/// A duration literal that breaks the unit ordering or overflow rules.
pub const E_BAD_DURATION: DiagCode = DiagCode("E0109");
/// A date, time-of-day or date-and-time literal that is not a real instant.
pub const E_BAD_DATE_TIME: DiagCode = DiagCode("E0110");
/// A `%` address that is not `%` location \[size\] digits (`.` digits)\*.
pub const E_BAD_DIRECT_ADDRESS: DiagCode = DiagCode("E0111");
/// An underscore in a numeric literal that does not separate two digits.
pub const E_MISPLACED_UNDERSCORE: DiagCode = DiagCode("E0112");
/// Comments or brackets nested deeper than the dialect allows.
pub const E_NESTING_TOO_DEEP: DiagCode = DiagCode("E0113");
/// A pragma `{ ... }` that is never closed.
pub const E_UNTERMINATED_PRAGMA: DiagCode = DiagCode("E0114");
/// A construct the dialect in force does not accept.
pub const E_DIALECT_REJECTS: DiagCode = DiagCode("E0115");
/// An identifier longer than salman accepts.
pub const E_IDENT_TOO_LONG: DiagCode = DiagCode("E0116");

/// Expressions, statements or operator chains nested deeper than the dialect
/// allows. Distinct from [`E_NESTING_TOO_DEEP`], which the lexer raises about
/// comments and brackets: this one is about the shape of the tree.
pub const E_PARSE_NESTING_TOO_DEEP: DiagCode = DiagCode("E0201");
/// A token the grammar does not allow where it was found.
pub const E_EXPECTED_TOKEN: DiagCode = DiagCode("E0202");
/// An expression was required and there was none.
pub const E_EXPECTED_EXPRESSION: DiagCode = DiagCode("E0203");
/// A statement was required and there was none.
pub const E_EXPECTED_STATEMENT: DiagCode = DiagCode("E0204");
/// A declaration was required and there was none.
pub const E_EXPECTED_DECLARATION: DiagCode = DiagCode("E0205");
/// A type was required and there was none.
pub const E_EXPECTED_TYPE: DiagCode = DiagCode("E0206");
/// A block that runs to the end of the file without its closing keyword.
pub const E_UNCLOSED_BLOCK: DiagCode = DiagCode("E0207");
/// Two `CASE` labels select the same value. A salman rule, not one salman could
/// verify in the standard; see `crate::parser`.
pub const E_DUPLICATE_CASE_LABEL: DiagCode = DiagCode("E0208");
/// Two `CASE` labels cover overlapping ranges. A salman rule; see
/// [`E_DUPLICATE_CASE_LABEL`].
pub const E_OVERLAPPING_CASE_LABELS: DiagCode = DiagCode("E0209");
/// A `FOR` body assigns to the loop's control variable. A salman rule.
pub const E_FOR_CONTROL_VARIABLE_ASSIGNED: DiagCode = DiagCode("E0210");
/// A word used as a name that the identifier rules refuse.
pub const E_BAD_NAME: DiagCode = DiagCode("E0211");

/// A literal prefix naming a type salman has not implemented, such as `LDT#`.
pub const U_UNSUPPORTED_LITERAL_PREFIX: DiagCode = DiagCode("U0101");

/// A type name that names no type.
pub const E_UNKNOWN_TYPE: DiagCode = DiagCode("E0301");
/// An identifier that names nothing in scope.
pub const E_UNKNOWN_NAME: DiagCode = DiagCode("E0302");
/// Two declarations of the same name in one scope.
pub const E_DUPLICATE_DECLARATION: DiagCode = DiagCode("E0303");
/// An array dimension whose bounds are inverted, empty or not constant
/// integers.
pub const E_BAD_ARRAY_BOUNDS: DiagCode = DiagCode("E0304");
/// A subrange whose bounds are inverted, or whose base type is not an integer.
pub const E_BAD_SUBRANGE: DiagCode = DiagCode("E0305");
/// A `STRING` length that is not a constant in the permitted range.
pub const E_BAD_STRING_LENGTH: DiagCode = DiagCode("E0306");
/// An expression that has to be a compile-time constant and is not.
pub const E_NOT_CONSTANT: DiagCode = DiagCode("E0307");
/// A `TYPE` declaration that, directly or through others, contains itself.
pub const E_RECURSIVE_TYPE: DiagCode = DiagCode("E0308");
/// A cycle in the call graph. salman rejects recursion; see `crate::sema`.
pub const E_RECURSIVE_CALL: DiagCode = DiagCode("E0309");
/// A field or parameter name a type does not have.
pub const E_UNKNOWN_MEMBER: DiagCode = DiagCode("E0310");
/// A function block's internal field, read or written from outside it. A salman
/// rule; see `crate::sema`.
pub const E_INTERNAL_FIELD: DiagCode = DiagCode("E0311");
/// An assignment whose left-hand side is not something that can be assigned to.
pub const E_NOT_ASSIGNABLE: DiagCode = DiagCode("E0312");
/// An assignment to a `VAR_INPUT` of the enclosing POU, or to a `CONSTANT`.
pub const E_NOT_WRITABLE: DiagCode = DiagCode("E0313");
/// A call whose callee is not a function or a function block instance.
pub const E_NOT_CALLABLE: DiagCode = DiagCode("E0314");
/// A positional argument in a function block call, which has no such form.
pub const E_POSITIONAL_FUNCTION_BLOCK_ARGUMENT: DiagCode = DiagCode("E0315");
/// A named argument whose name is not a parameter of the callee.
pub const E_UNKNOWN_PARAMETER: DiagCode = DiagCode("E0316");
/// A call with the wrong number of arguments.
pub const E_WRONG_ARGUMENT_COUNT: DiagCode = DiagCode("E0317");
/// A function block call used where a value is required. It produces none.
pub const E_FUNCTION_BLOCK_HAS_NO_VALUE: DiagCode = DiagCode("E0318");
/// A call to something callable in principle but not in this form: a `PROGRAM`,
/// or a `FUNCTION_BLOCK` type rather than one of its instances.
pub const E_WRONG_CALL_TARGET: DiagCode = DiagCode("E0319");
/// `EXIT` or `CONTINUE` outside any loop.
pub const E_JUMP_OUTSIDE_LOOP: DiagCode = DiagCode("E0320");
/// A `CONFIGURATION`, `RESOURCE`, `TASK` or program instance salman cannot make
/// sense of.
pub const E_BAD_CONFIGURATION: DiagCode = DiagCode("E0321");
/// One call that mixes positional and named arguments.
pub const E_MIXED_ARGUMENT_FORMS: DiagCode = DiagCode("E0322");
/// Field access on a type that has no fields.
pub const E_NOT_AN_AGGREGATE: DiagCode = DiagCode("E0323");
/// A variable declared with a name the calling convention reserves: `EN`, `ENO`.
pub const E_RESERVED_PARAMETER_NAME: DiagCode = DiagCode("E0324");
/// One parameter given an argument twice in the same call.
pub const E_DUPLICATE_ARGUMENT: DiagCode = DiagCode("E0325");

/// A value that cannot be assigned to a target of the type it was given.
pub const E_TYPE_MISMATCH: DiagCode = DiagCode("E0401");
/// Two operands of one operator with no common type.
pub const E_NO_COMMON_TYPE: DiagCode = DiagCode("E0402");
/// An operand outside the generic type its operator accepts.
pub const E_OPERAND_OUTSIDE_DOMAIN: DiagCode = DiagCode("E0403");
/// A literal that does not fit the type it was given.
pub const E_LITERAL_DOES_NOT_FIT: DiagCode = DiagCode("E0404");
/// An `IF`, `WHILE` or `REPEAT` condition that is not `BOOL`.
pub const E_CONDITION_NOT_BOOL: DiagCode = DiagCode("E0405");
/// A subscript applied to something that is not an array.
pub const E_NOT_AN_ARRAY: DiagCode = DiagCode("E0406");
/// A different number of subscripts from the number of declared dimensions.
pub const E_WRONG_SUBSCRIPT_COUNT: DiagCode = DiagCode("E0407");
/// A subscript that is not an integer.
pub const E_SUBSCRIPT_NOT_INTEGER: DiagCode = DiagCode("E0408");
/// A constant subscript outside the declared bounds.
pub const E_SUBSCRIPT_OUT_OF_BOUNDS: DiagCode = DiagCode("E0409");
/// A `CASE` selector that is neither an integer nor an enumeration.
pub const E_CASE_SELECTOR_TYPE: DiagCode = DiagCode("E0410");
/// A `CASE` label whose type does not match the selector's.
pub const E_CASE_LABEL_TYPE: DiagCode = DiagCode("E0411");
/// A `FOR` control variable that is not a writable local integer.
pub const E_FOR_CONTROL_VARIABLE: DiagCode = DiagCode("E0412");
/// A `FOR` bound or step that is not an integer.
pub const E_FOR_BOUND_TYPE: DiagCode = DiagCode("E0413");
/// A `FOR` step that is constant zero, which never terminates.
pub const E_FOR_STEP_ZERO: DiagCode = DiagCode("E0414");
/// Division by a constant zero, found before the program ever runs.
pub const E_CONSTANT_DIVISION_BY_ZERO: DiagCode = DiagCode("E0415");

/// A construct salman parses far enough to name, and does not implement.
pub const U_UNIMPLEMENTED_CONSTRUCT: DiagCode = DiagCode("U0201");

/// References: `REF_TO`, `^` and the assignment attempt `?=`. salman implements
/// none of them.
pub const U_REFERENCES: DiagCode = DiagCode("U0301");

/// A duration literal finer than one nanosecond, whose tail was truncated.
pub const W_DURATION_TRUNCATED: DiagCode = DiagCode("W0101");
/// Two consecutive underscores in an identifier.
pub const W_CONSECUTIVE_UNDERSCORES: DiagCode = DiagCode("W0102");

/// An unparenthesised unary operand of `**`, where dialects disagree about
/// which binds tighter.
pub const W_POWER_OPERAND_BINDING: DiagCode = DiagCode("W0201");

/// A `FUNCTION` that can finish without ever assigning its result. A warning
/// rather than an error; see `crate::sema`.
pub const W_FUNCTION_RESULT_NOT_ASSIGNED: DiagCode = DiagCode("W0301");
/// A `FOR` limit outside the control variable's subrange, which the loop will
/// step into unless an `EXIT` leaves it first.
pub const W_FOR_LIMIT_OUTSIDE_SUBRANGE: DiagCode = DiagCode("W0302");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_codes_are_unique() {
        let all = [
            E_UNEXPECTED_CHARACTER,
            E_UNTERMINATED_COMMENT,
            E_UNTERMINATED_STRING,
            E_BAD_ESCAPE,
            E_BAD_RADIX,
            E_BAD_DIGIT,
            E_LITERAL_OUT_OF_RANGE,
            E_BAD_REAL,
            E_BAD_DURATION,
            E_BAD_DATE_TIME,
            E_BAD_DIRECT_ADDRESS,
            E_MISPLACED_UNDERSCORE,
            E_NESTING_TOO_DEEP,
            E_UNTERMINATED_PRAGMA,
            E_DIALECT_REJECTS,
            E_IDENT_TOO_LONG,
            E_PARSE_NESTING_TOO_DEEP,
            E_EXPECTED_TOKEN,
            E_EXPECTED_EXPRESSION,
            E_EXPECTED_STATEMENT,
            E_EXPECTED_DECLARATION,
            E_EXPECTED_TYPE,
            E_UNCLOSED_BLOCK,
            E_DUPLICATE_CASE_LABEL,
            E_OVERLAPPING_CASE_LABELS,
            E_FOR_CONTROL_VARIABLE_ASSIGNED,
            E_BAD_NAME,
            E_UNKNOWN_TYPE,
            E_UNKNOWN_NAME,
            E_DUPLICATE_DECLARATION,
            E_BAD_ARRAY_BOUNDS,
            E_BAD_SUBRANGE,
            E_BAD_STRING_LENGTH,
            E_NOT_CONSTANT,
            E_RECURSIVE_TYPE,
            E_RECURSIVE_CALL,
            E_UNKNOWN_MEMBER,
            E_INTERNAL_FIELD,
            E_NOT_ASSIGNABLE,
            E_NOT_WRITABLE,
            E_NOT_CALLABLE,
            E_POSITIONAL_FUNCTION_BLOCK_ARGUMENT,
            E_RESERVED_PARAMETER_NAME,
            E_DUPLICATE_ARGUMENT,
            E_UNKNOWN_PARAMETER,
            E_WRONG_ARGUMENT_COUNT,
            E_FUNCTION_BLOCK_HAS_NO_VALUE,
            E_WRONG_CALL_TARGET,
            E_JUMP_OUTSIDE_LOOP,
            E_BAD_CONFIGURATION,
            E_MIXED_ARGUMENT_FORMS,
            E_NOT_AN_AGGREGATE,
            E_TYPE_MISMATCH,
            E_NO_COMMON_TYPE,
            E_OPERAND_OUTSIDE_DOMAIN,
            E_LITERAL_DOES_NOT_FIT,
            E_CONDITION_NOT_BOOL,
            E_NOT_AN_ARRAY,
            E_WRONG_SUBSCRIPT_COUNT,
            E_SUBSCRIPT_NOT_INTEGER,
            E_SUBSCRIPT_OUT_OF_BOUNDS,
            E_CASE_SELECTOR_TYPE,
            E_CASE_LABEL_TYPE,
            E_FOR_CONTROL_VARIABLE,
            E_FOR_BOUND_TYPE,
            E_FOR_STEP_ZERO,
            E_CONSTANT_DIVISION_BY_ZERO,
            U_UNSUPPORTED_LITERAL_PREFIX,
            U_UNIMPLEMENTED_CONSTRUCT,
            U_REFERENCES,
            W_DURATION_TRUNCATED,
            W_CONSECUTIVE_UNDERSCORES,
            W_POWER_OPERAND_BINDING,
            W_FUNCTION_RESULT_NOT_ASSIGNED,
            W_FOR_LIMIT_OUTSIDE_SUBRANGE,
        ];
        let mut codes: Vec<&str> = all.iter().map(|c| c.0).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count, "duplicate diagnostic code");
    }
}
