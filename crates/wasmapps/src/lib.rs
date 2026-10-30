//! A demo app as a wasm module for the renderer's wasm sessions (`SPEC.md`,
//! section 4): `receive` runs the app on the lines it is given and passes
//! what the app wrote to the renderer's `hyperb.send`. An app that ticks
//! asks the renderer to `wake` it, again after every tick, for as long as
//! it wants ticks.

#[cfg(feature = "billsplit")]
type App = demoapps::billsplit::BillSplit;
#[cfg(feature = "chart")]
type App = demoapps::chart::Chart;
#[cfg(feature = "dbclient")]
type App = demoapps::dbclient::DbClient;
#[cfg(feature = "finance")]
type App = demoapps::finance::Finance;
#[cfg(feature = "grammar")]
type App = demoapps::grammar::Grammar;
#[cfg(feature = "minecraft")]
type App = demoapps::minecraft::Minecraft;
#[cfg(feature = "pomodoro")]
type App = demoapps::pomodoro::Pomodoro;
#[cfg(feature = "qr")]
type App = demoapps::qr::Qr;
#[cfg(feature = "restaurant")]
type App = demoapps::restaurant::Restaurant;
#[cfg(feature = "saas")]
type App = demoapps::saas::Saas;
#[cfg(feature = "smarthome")]
type App = demoapps::smarthome::SmartHome;
#[cfg(feature = "textstats")]
type App = demoapps::textstats::TextStats;
#[cfg(feature = "therapist")]
type App = demoapps::therapist::Therapist;

#[cfg(any(
    feature = "billsplit",
    feature = "chart",
    feature = "dbclient",
    feature = "finance",
    feature = "grammar",
    feature = "minecraft",
    feature = "pomodoro",
    feature = "qr",
    feature = "restaurant",
    feature = "saas",
    feature = "smarthome",
    feature = "textstats",
    feature = "therapist"
))]
mod glue {
    use hypereffect::server::Server;
    use std::cell::{Cell, RefCell};

    #[cfg(target_arch = "wasm32")]
    #[link(wasm_import_module = "hyperb")]
    unsafe extern "C" {
        fn send(ptr: *const u8, len: u32);
        fn now() -> f64;
        fn wake_in(ms: i32);
    }

    #[cfg(not(target_arch = "wasm32"))]
    unsafe fn send(_: *const u8, _: u32) {}
    #[cfg(not(target_arch = "wasm32"))]
    unsafe fn now() -> f64 {
        0.0
    }
    #[cfg(not(target_arch = "wasm32"))]
    unsafe fn wake_in(_: i32) {}

    thread_local! {
        static SERVER: RefCell<Server<super::App>> = RefCell::new(Server::new());
        /// The tick interval of the pending wake-up, if one is.
        static WAKING: Cell<Option<u32>> = const { Cell::new(None) };
    }

    /// A buffer for the renderer to write a line into; `receive` takes it back.
    #[unsafe(no_mangle)]
    pub extern "C" fn alloc(len: u32) -> *mut u8 {
        Box::into_raw(vec![0u8; len as usize].into_boxed_slice()) as *mut u8
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn receive(ptr: *mut u8, len: u32) {
        // SAFETY: `ptr` came from `alloc(len)` and is received once.
        let input = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len as usize)) };
        SERVER.with_borrow_mut(|s| {
            s.receive(&input);
            flush(s);
        });
    }

    /// The wake-up `flush` asked for.
    #[unsafe(no_mangle)]
    pub extern "C" fn wake() {
        WAKING.set(None);
        SERVER.with_borrow_mut(|s| {
            // SAFETY: an import without arguments.
            s.tick(unsafe { now() });
            flush(s);
        });
    }

    /// Sends what the app wrote, and asks for the wake-up of its next tick,
    /// or cancels the pending one if it no longer ticks at that interval.
    fn flush(s: &mut Server<super::App>) {
        let out = s.output().take();
        if !out.is_empty() {
            // SAFETY: the buffer is valid for the call.
            unsafe { send(out.as_ptr(), out.len() as u32) };
        }
        let want = s.tick_interval();
        if want != WAKING.get() {
            let ms = want.map_or(-1, |ms| ms.min(i32::MAX as u32) as i32);
            // SAFETY: an import taking a number.
            unsafe { wake_in(ms) };
            WAKING.set(want);
        }
    }

    /// Random numbers for Harper's models (the `custom` backend of
    /// getrandom): the module has no source of entropy, so they come from the
    /// clock, through SplitMix64. Not for secrets.
    #[cfg(feature = "grammar")]
    #[unsafe(no_mangle)]
    unsafe extern "Rust" fn __getrandom_v03_custom(
        dest: *mut u8,
        len: usize,
    ) -> Result<(), getrandom::Error> {
        thread_local! {
            static STATE: Cell<u64> = Cell::new(unsafe { now() }.to_bits());
        }
        // SAFETY: getrandom passes a buffer of `len` bytes.
        let dest = unsafe { std::slice::from_raw_parts_mut(dest, len) };
        for chunk in dest.chunks_mut(8) {
            let x = STATE.get().wrapping_add(0x9e37_79b9_7f4a_7c15);
            STATE.set(x);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            chunk.copy_from_slice(&z.to_le_bytes()[..chunk.len()]);
        }
        Ok(())
    }
}
