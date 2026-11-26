// SPDX-License-Identifier: AGPL-3.0-or-later
import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace         = "com.domoconf.comm"
    compileSdk        = 35

    defaultConfig {
        applicationId  = "com.domoconf.comm"
        minSdk         = 26          // Android 8.0
        targetSdk      = 35
        versionCode    = 1
        versionName    = "1.0.0"
    }

    // Signing key of the published application. Its file is read from outside the repository: the path comes from
    // DOMOCOMM_SIGNING (a properties file with storeFile, storePassword, keyAlias, keyPassword), by default
    // ~/.config/domocomm/signing.properties. Without it the release build stays unsigned (make android-apk refuses to run).
    val signingFile = File(
        System.getenv("DOMOCOMM_SIGNING") ?: "${System.getProperty("user.home")}/.config/domocomm/signing.properties"
    )
    val releaseSigning = if (signingFile.isFile) {
        val p = Properties().apply { signingFile.inputStream().use { load(it) } }
        signingConfigs.create("release") {
            storeFile     = File(p.getProperty("storeFile"))
            storePassword = p.getProperty("storePassword")
            keyAlias      = p.getProperty("keyAlias")
            keyPassword   = p.getProperty("keyPassword")
        }
    } else null

    buildTypes {
        release {
            signingConfig = releaseSigning
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        viewBinding = true   // replaces findViewById
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.appcompat)
    implementation(libs.material)
    implementation(libs.androidx.constraintlayout)

    // OkHttp: WebSocket client
    implementation(libs.okhttp)

    // Gson: JSON
    implementation(libs.gson)
}
