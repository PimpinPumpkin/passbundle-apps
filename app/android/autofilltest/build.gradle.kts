// A sign-in form for the autofill test (tools/android-autofill-test.sh): an
// app PassBundle fills, the way it fills any other. Never shipped.
plugins {
    id("com.android.application")
}

android {
    namespace = "com.passbundle.autofilltest"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.passbundle.autofilltest"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "1"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

dependencies {
    // Android's Credential Manager, as an app with passkeys uses it.
    implementation("androidx.credentials:credentials:1.6.0")
}
