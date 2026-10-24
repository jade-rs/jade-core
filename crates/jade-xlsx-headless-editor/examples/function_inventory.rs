//! Machine-readable function names registered by the actual calculation engine.
fn main() {
    let names = ironcalc_base::supported_function_names();
    println!("{}", serde_json::to_string_pretty(&names).unwrap());
}
