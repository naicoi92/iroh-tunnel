//! Windows trust policy for files a LocalSystem service reads.
//!
//! `model` is pure and compiles on every platform; the Win32 adapter that
//! feeds it real descriptors arrives in a later task.
#![cfg_attr(not(windows), allow(dead_code))]
// Windows too until Task 5 adds the consumer (the Win32 adapter).
#![cfg_attr(windows, allow(dead_code))]

pub(crate) mod model;
