//! The functions the host calls on the guest module.
//!
//! The host writes a frame's input into a buffer from [`pg_alloc`] and passes it to
//! [`pg_frame`], which takes the buffer over and frees it.

use std::alloc::{alloc, dealloc, Layout};

fn layout(len: usize) -> Layout {
    Layout::from_size_align(len.max(1), 1).expect("byte buffer layout")
}

/// Reserves `len` bytes in the guest for the host to write frame input into.
#[no_mangle]
pub extern "C" fn pg_alloc(len: usize) -> *mut u8 {
    // SAFETY: the layout has a non-zero size.
    unsafe { alloc(layout(len)) }
}

/// Releases a buffer from [`pg_alloc`] that was never passed to [`pg_frame`].
///
/// # Safety
/// `ptr` comes from [`pg_alloc`] with the same `len` and has not been freed.
#[no_mangle]
pub unsafe extern "C" fn pg_free(ptr: *mut u8, len: usize) {
    dealloc(ptr, layout(len));
}

/// Runs one frame.
///
/// Returns a pointer to a `u32` byte count followed by that many bytes of encoded output. It
/// stays valid until the next call.
///
/// # Safety
/// `ptr` comes from [`pg_alloc`] with the same `len` and holds encoded frame input.
#[no_mangle]
pub unsafe extern "C" fn pg_frame(ptr: *mut u8, len: usize) -> *const u8 {
    let input = std::slice::from_raw_parts(ptr, len);
    let output = crate::frame(input);
    dealloc(ptr, layout(len));
    output
}
