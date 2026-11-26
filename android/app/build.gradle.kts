// SPDX-License-Identifier: AGPL-3.0-or-later
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
        versionName    = "1.0"
    }

    buildTypes {
        release {
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
