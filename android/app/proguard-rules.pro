# JNA 与 UniFFI 通过反射/本地库调用，保留其类。
-keep class com.sun.jna.** { *; }
-keep class uniffi.** { *; }
-dontwarn java.awt.*
