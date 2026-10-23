plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.passbundle.app"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "com.passbundle.app"
        // Android 10, the oldest the apps support (docs/phones.md).
        minSdk = 29
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
            // Classes only the core calls, by name through JNI, so shrinking
            // must keep them (proguard-rules.pro).
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

// The Kotlin half of the core's certificate checks, at the version in the
// core's Cargo.lock: tools/android-core.sh fetches it into ../rustls-maven and
// checks it against ../rustls-platform-verifier.sha256.
val rustlsVerifierVersion: String =
    run {
        val lock = rootProject.file("../../core/Cargo.lock").readLines()
        val at = lock.indexOfFirst { it.trim() == "name = \"rustls-platform-verifier-android\"" }
        require(at >= 0) { "rustls-platform-verifier-android isn't in core/Cargo.lock" }
        Regex("\"([^\"]+)\"").find(lock[at + 1])!!.groupValues[1]
    }

dependencies {
    implementation("org.rustls:rustls-platform-verifier:$rustlsVerifierVersion")
    // Android's Credential Manager, for passkeys (PasskeyProvider.kt). Apache 2.0.
    implementation("androidx.credentials:credentials:1.6.0")
}
