# E-Chat
Client what combines telegram and matrix and also supports all platforms (including web :O)


# Dev
For building wasm you need specify `RUSTFLAGS='--cfg getrandom_backend="wasm_js"'`
For building native you need remove and make just `RUSTFLAGS=`