//! The common read-only dereference of checked mutable state handles.

use std::ops::Deref;

/// A mutable state value that can be read, but not overwritten, through the handle.
///
/// Specialized aliases expose only the checked operations of their value type. The constructor and mutable field are
/// crate-private, and there is deliberately no `DerefMut` implementation.
#[doc(hidden)]
#[derive(Debug)]
pub struct CheckedMut<'a, T> {
    /// The value; never given out as a `&mut` outside the state module.
    pub(crate) value: &'a mut T,
}

impl<'a, T> CheckedMut<'a, T> {
    /// A checked handle to `value`.
    pub(crate) const fn new(value: &'a mut T) -> Self {
        Self { value }
    }
}

impl<T> Deref for CheckedMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}
