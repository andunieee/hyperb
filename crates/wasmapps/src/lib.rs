//! A demo app as a wasm module for the renderer's wasm sessions (`SPEC.md`,
//! section 4): `receive` runs the app on the lines it is given and passes
//! what the app wrote to the renderer's `hyperb.send`.

#[cfg(feature = "billsplit")]
type App = demoapps::billsplit::BillSplit;
#[cfg(feature = "chart")]
type App = demoapps::chart::Chart;
#[cfg(feature = "dbclient")]
type App = demoapps::dbclient::DbClient;
#[cfg(feature = "finance")]
type App = demoapps::finance::Finance;
#[cfg(feature = "minecraft")]
type App = demoapps::minecraft::Minecraft;
#[cfg(feature = "qr")]
type App = demoapps::qr::Qr;
#[cfg(feature = "textstats")]
type App = demoapps::textstats::TextStats;

#[cfg(any(
    feature = "billsplit",
    feature = "chart",
    feature = "dbclient",
    feature = "finance",
    feature = "minecraft",
    feature = "qr",
    feature = "textstats"
))]
mod glue {
    use hypereffect::server::Server;
    use std::cell::RefCell;

    #[cfg(target_arch = "wasm32")]
    #[link(wasm_import_module = "hyperb")]
    extern "C" {
        fn send(ptr: *const u8, len: u32);
    }

    #[cfg(not(target_arch = "wasm32"))]
    unsafe fn send(_: *const u8, _: u32) {}

    thread_local! {
        static SERVER: RefCell<Server<super::App>> = RefCell::new(Server::new());
    }

    /// A buffer for the renderer to write a line into; `receive` takes it back.
    #[no_mangle]
    pub extern "C" fn alloc(len: u32) -> *mut u8 {
        Box::into_raw(vec![0u8; len as usize].into_boxed_slice()) as *mut u8
    }

    #[no_mangle]
    pub extern "C" fn receive(ptr: *mut u8, len: u32) {
        // SAFETY: `ptr` came from `alloc(len)` and is received once.
        let input = unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len as usize))
        };
        let out = SERVER.with_borrow_mut(|s| {
            s.receive(&input);
            s.output().take()
        });
        if !out.is_empty() {
            // SAFETY: the buffer is valid for the call.
            unsafe { send(out.as_ptr(), out.len() as u32) };
        }
    }
}
