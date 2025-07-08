use std::{
    error::Error, fmt::{Debug, Display}, ops::Deref, sync::Arc
};

pub struct SharedError<E> {
    inner: Arc<E>,
}

impl<E> Clone for SharedError<E> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<E: std::error::Error> SharedError<E> {
    pub fn new(inner: E) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn inner(&self) -> &E {
        &self.inner
    }

    /// Attempts to take the error out of the Arc
    /// This will only succeed if the Arc has exactly one strong reference
    pub fn take(self) -> Result<E, Self> {
        Arc::try_unwrap(self.inner).map_err(|e| Self { inner: e })
    }
}

impl<E: std::error::Error + Debug + Display> Error for SharedError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.inner.source()
    }
}

impl<E: Display> Display for SharedError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.inner, f)
    }
}

impl<E: Debug> Debug for SharedError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.inner, f)
    }
}

impl<E> From<E> for SharedError<E> {
    fn from(inner: E) -> Self {
        let inner = inner.into();
        SharedError { inner }
    }
}

impl<E> Deref for SharedError<E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}