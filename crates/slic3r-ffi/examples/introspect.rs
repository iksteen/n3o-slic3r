// cargo run --example introspect
//
// Demonstrates the runtime option-def introspection API. No model file needed.

use slic3r_ffi::{init, option_defs, version, OptType};

fn main() {
    init(None, 3).expect("init failed");
    println!("{}", version());

    let defs = option_defs();
    println!("total options: {}", defs.len());

    let mut by_type: std::collections::BTreeMap<String, usize> = Default::default();
    for d in &defs {
        *by_type.entry(format!("{:?}", d.ty)).or_default() += 1;
    }
    println!("\nby type:");
    for (ty, count) in &by_type {
        println!("  {ty:<20} {count}");
    }

    println!("\nfirst 5 enum options with their valid values:");
    let mut shown = 0;
    for d in &defs {
        if d.ty == OptType::Enum && shown < 5 {
            println!("  {} ({})", d.key, d.label.as_deref().unwrap_or(""));
            for (k, l) in d.enum_values.iter().zip(
                d.enum_labels
                    .iter()
                    .chain(std::iter::repeat(&String::new())),
            ) {
                println!("      {k:<24} {l}");
            }
            shown += 1;
        }
    }

    println!("\nlayer_height detail:");
    let lh = slic3r_ffi::option_def("layer_height").expect("layer_height missing");
    println!("  {lh:#?}");
}
