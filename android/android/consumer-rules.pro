# Keep the JNA and UniFFI binding classes (release builds use R8, AUDIT S-12)
-keep class com.sun.jna.** { *; }
-keep class com.roadmesh.core.** { *; }
-dontwarn java.awt.**
