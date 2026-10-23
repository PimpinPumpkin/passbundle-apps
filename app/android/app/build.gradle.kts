import java.util.Properties

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

// The key release builds are signed with (README, "Android test releases"):
// from PASSBUNDLE_KEYSTORE_PATH, PASSBUNDLE_KEYSTORE_PASSWORD, and
// PASSBUNDLE_KEY_ALIAS, as the release workflow sets them, or from a
// properties file outside the repository that PASSBUNDLE_KEY_PROPERTIES names
// (storeFile, storePassword, keyAlias, keyPassword), for a build on a
// computer. With neither, release builds are signed with the debug key, so
// `flutter run --release` and forks' builds work. Nothing here is printed.
class ReleaseKey(val store: File, val storePassword: String, val alias: String, val keyPassword: String)

val releaseKey: ReleaseKey? =
    run {
        val fromFile = System.getenv("PASSBUNDLE_KEY_PROPERTIES")?.takeIf { it.isNotBlank() }
        val fromEnv = System.getenv("PASSBUNDLE_KEYSTORE_PATH")?.takeIf { it.isNotBlank() }
        when {
            fromFile != null -> {
                val file = File(fromFile)
                require(file.isFile) { "PASSBUNDLE_KEY_PROPERTIES names no file: $fromFile" }
                val p = Properties().apply { file.inputStream().use { load(it) } }
                fun need(key: String) = requireNotNull(p.getProperty(key)?.takeIf { it.isNotBlank() }) { "$fromFile has no $key" }
                ReleaseKey(File(need("storeFile")), need("storePassword"), need("keyAlias"), need("keyPassword"))
            }
            fromEnv != null -> {
                val password = requireNotNull(System.getenv("PASSBUNDLE_KEYSTORE_PASSWORD")?.takeIf { it.isNotEmpty() }) {
                    "PASSBUNDLE_KEYSTORE_PATH is set without PASSBUNDLE_KEYSTORE_PASSWORD"
                }
                // The store's and the key's password are the same.
                ReleaseKey(File(fromEnv), password, System.getenv("PASSBUNDLE_KEY_ALIAS")?.takeIf { it.isNotBlank() } ?: "passbundle-test", password)
            }
            else -> null
        }?.also { require(it.store.isFile) { "No keystore at ${it.store}" } }
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

    signingConfigs {
        releaseKey?.let { key ->
            create("release") {
                storeFile = key.store
                storePassword = key.storePassword
                keyAlias = key.alias
                keyPassword = key.keyPassword
            }
        }
    }

    buildTypes {
        release {
            // The release key when there is one (above), else the debug key.
            signingConfig = signingConfigs.getByName(if (releaseKey != null) "release" else "debug")
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
    // The updater's own logic, tested on the computer (src/test). Eclipse Public License 1.0.
    testImplementation("junit:junit:4.13.2")
}
