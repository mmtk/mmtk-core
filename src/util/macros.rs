// Workaround for Rust not having constant if-expressions
// I am so, so sorry
#[macro_export]
macro_rules! if_then_else_zero_usize {
    ($cond:expr, $then_expr:expr) => ((-($cond as isize) as usize) & $then_expr);
}

#[macro_export]
macro_rules! if_then_else_usize {
    ($cond:expr, $then_expr:expr, $else_expr:expr) => ({
        if_then_else_zero_usize!($cond, $then_expr) | if_then_else_zero_usize!(!$cond, $else_expr)
    });
}

#[test]
fn test_if_then_else_zero_usize() {
    for i in 0 .. 101usize {
        assert_eq!(if_then_else_zero_usize!(true, i), i);
        assert_eq!(if_then_else_zero_usize!(false, i), 0);
    }
}

#[test]
fn test_if_then_else_usize() {
    for i in 0 .. 101usize {
        for j in 1 .. 101usize {
            assert_eq!(if_then_else_usize!(true, i, j), i);
            assert_eq!(if_then_else_usize!(false, i, j), j);
        }
    }
}