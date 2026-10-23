# Release builds shrink and rename code they think nothing uses (R8). The
# core reaches these classes only by name through JNI, which R8 can't see,
# so without these rules a release build drops them: Android's certificate
# checks fail ("failed to call native verifier") and the session isn't kept.
# tools/android-release-check.sh checks a release build still has them.

# The Kotlin half of the core's certificate checks (rustls-platform-verifier).
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }

# The app's classes the core calls (core/src/android.rs) or registers with.
-keep class com.passbundle.app.Core { *; }
-keep class com.passbundle.app.Keystore { *; }
-keep class com.passbundle.app.Biometric { *; }
