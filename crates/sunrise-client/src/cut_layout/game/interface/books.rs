#![cfg(test)]
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "tests/book_escape_tests.rs"]
pub(crate) mod book_escape_tests;

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "tests/sign_book_tests.rs"]
pub(crate) mod sign_book_tests;
