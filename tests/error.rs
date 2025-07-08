use batched::{batched, error::SharedError};

#[test]
fn error_type_works() {
    fn _error() -> Result<(), SharedError<std::io::Error>> {
        // Purely for type checking
        std::fs::write("/tmp/1234", "1234")?;
        Ok(())
    }
}

#[test]
fn auto_into_error_type_works() {
    #[batched(window = 100, limit = 1000)]
    fn _error(_v: Vec<bool>) -> Result<(), SharedError<anyhow::Error>> {
        // Purely for type checking
        std::fs::write("/tmp/1234", "1234")?;
        Ok(())
    }
}
