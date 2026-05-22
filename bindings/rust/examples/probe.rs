// Minimal segfault probe — exercises each call in isolation.

fn main() {
    println!("step 1: just print");
    println!("step 2: version (no init)");
    let v = slic3r_ffi::version();
    println!("  version = {v}");
    println!("step 3: init");
    println!("  step 3a: init with log_level=2");
    slic3r_ffi::init(None, 2).expect("init");
    println!("  init ok");
    println!("step 4: option_def_count");
    // SAFETY: probing the raw API directly.
    let c = unsafe { slic3r_ffi::sys::slic3r_option_def_count() };
    println!("  count = {c}");
}
