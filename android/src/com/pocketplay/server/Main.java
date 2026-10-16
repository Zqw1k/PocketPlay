package com.pocketplay.server;

import java.util.Arrays;

/** Minimal app_process entry point; the device engine lives in native Rust. */
public final class Main {
    private Main() { }
    private static native void run(String[] args);

    public static void main(String[] args) {
        if (args.length < 1 || !args[0].startsWith("/data/local/tmp/pocketplay-")) {
            throw new IllegalArgumentException("Expected the session's native library path");
        }
        System.out.println("PocketPlay loading native engine");
        System.load(args[0]);
        System.out.println("PocketPlay entering native engine");
        run(Arrays.copyOfRange(args, 1, args.length));
    }
}
