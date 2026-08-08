//! A wrapper that prevents accidental logging of sensitive values. `Secret<T>`
//! deliberately does not implement `Debug`/`Display`; the only way to read
//! the inner value is the explicit `expose()` call, so a `grep -rn expose`
//! finds every place secret material is actually touched.

pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &T {
        &self.0
    }
}

// Intentionally no Debug/Display impl.
