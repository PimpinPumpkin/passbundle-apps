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
-keep class com.passbundle.app.SecurityKeys { *; }

# UnifiedPush's connector (Push.kt) opens push messages with Google's Tink,
# whose keysets are protocol buffers read by reflection, and whose key
# downloader (never used here) needs Google's HTTP client, which isn't in the
# app: keep the first, and don't stop at the missing classes of the second.
-keep class com.google.crypto.tink.proto.** { *; }
-keepclassmembers class * extends com.google.protobuf.GeneratedMessageLite { <fields>; }
-keepclassmembers class * extends com.google.protobuf.GeneratedMessage { <fields>; }
-dontwarn com.google.api.client.http.**
-dontwarn org.joda.time.**
