fn main() {
    cc::Build::new()
        .file("src/c/pqueue.c")
        .file("src/c/reactor.c")
        .file("src/c/HelloWorld.c")
        .flag("-w")
        .static_flag(true)
        .compile("lingua_franca")
}
