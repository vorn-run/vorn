pub fn f() {
    // "task.secretly" in a comment is fine
    use vorn_extension_sdk::guest;
    let _ = guest::x;
    let _ = "task.secretly";
}
