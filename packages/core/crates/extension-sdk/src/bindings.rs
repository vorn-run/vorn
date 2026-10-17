//! The extension world of `wit/extension.wit`, as wit-bindgen generates it
//! for a guest. Authors use the crate's types and [`crate::export!`] instead.

wit_bindgen::generate!({
    world: "extension",
    path: "../extensions/wit/extension.wit",
    pub_export_macro: true,
    export_macro_name: "__export_extension",
    default_bindings_module: "vorn_extension_sdk::bindings",
});
