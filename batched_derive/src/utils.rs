use syn::{Expr, ExprLit, Lit};

pub fn expr_to_u64(expr: &Expr) -> Option<u64> {
    if let Expr::Lit(ExprLit { lit: Lit::Int(lit_int), .. }) = expr {
        lit_int.base10_parse::<u64>().ok()
    } else {
        None
    }
}
