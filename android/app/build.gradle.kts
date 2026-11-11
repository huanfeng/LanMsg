import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

// 版本号唯一来源：根 Cargo.toml 的 [workspace.package].version（versionCode = major*10000 + minor*100 + patch）
val workspaceVersion: String = rootProject.file("../Cargo.toml").readLines()
    .dropWhile { it.trim() != "[workspace.package]" }
    .drop(1)
    .takeWhile { !it.trim().startsWith("[") }
    .firstNotNullOfOrNull { Regex("""^\s*version\s*=\s*"([^"]+)"""").find(it)?.groupValues?.get(1) }
    ?: throw GradleException("cannot read [workspace.package].version from Cargo.toml")
val versionParts = Regex("""^(\d+)\.(\d+)\.(\d+)""").find(workspaceVersion)?.groupValues
    ?: throw GradleException("workspace version is not x.y.z: $workspaceVersion")

android {
    namespace = "name.apk.lanmsg"
    compileSdk = 35

    defaultConfig {
        applicationId = "name.apk.lanmsg"
        minSdk = 26
        targetSdk = 35
        versionCode = versionParts[1].toInt() * 10000 + versionParts[2].toInt() * 100 + versionParts[3].toInt()
        versionName = workspaceVersion
        // 只保留实际提供原生库的三种 ABI，去掉 JNA 带入的 mips/armeabi/x86 等
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }

    // release 签名读取 android/keystore.properties；缺失时回退 debug 签名，便于 CI 出包测试
    val keystoreFile = rootProject.file("keystore.properties")
    val keystoreProps = Properties().apply {
        if (keystoreFile.exists()) keystoreFile.inputStream().use { load(it) }
    }
    signingConfigs {
        if (keystoreFile.exists()) {
            val required = listOf("storeFile", "storePassword", "keyAlias", "keyPassword")
            val missing = required.filter { keystoreProps.getProperty(it).isNullOrBlank() }
            if (missing.isNotEmpty()) {
                throw GradleException("keystore.properties is missing keys: ${missing.joinToString(", ")}")
            }
            create("release") {
                storeFile = rootProject.file(keystoreProps.getProperty("storeFile"))
                storePassword = keystoreProps.getProperty("storePassword")
                keyAlias = keystoreProps.getProperty("keyAlias")
                keyPassword = keystoreProps.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            signingConfig = if (keystoreFile.exists()) {
                signingConfigs.getByName("release")
            } else {
                logger.warn("WARNING: keystore.properties not found; release build is signed with the debug key")
                signingConfigs.getByName("debug")
            }
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
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
        compose = true
        buildConfig = true
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
    }

    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons.extended)
    implementation(libs.activity.compose)
    implementation(libs.navigation.compose)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.lifecycle.process)
    implementation(libs.lifecycle.service)
    implementation(libs.work.runtime.ktx)
    implementation(libs.datastore.preferences)
    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.view)
    implementation(libs.zxing.core)
    implementation(libs.coil.compose)
    implementation(libs.coil.video)
    implementation("net.java.dev.jna:jna:${libs.versions.jna.get()}@aar")
    implementation(libs.coroutines.android)
    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    testImplementation(libs.junit)
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
    testImplementation(libs.robolectric)
    testImplementation(libs.turbine)
    testImplementation(libs.coroutines.test)
    testImplementation(libs.androidx.test.core.ktx)
    testImplementation(libs.androidx.test.ext.junit.ktx)
}
