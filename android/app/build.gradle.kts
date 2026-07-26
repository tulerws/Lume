import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.ksp)
    alias(libs.plugins.hilt)
    alias(libs.plugins.kotlin.serialization)
}

/**
 * Credenciais de desenvolvimento, lidas de `local.properties`.
 *
 * Espelham o `LUME_REMOTE_DEV` do desktop, e existem pelo mesmo motivo: o
 * pareamento depende do QR, que depende da câmera, que ainda não existe neste
 * cliente. `docs/REMOTE-CONTROL.md` cria a variável do lado Rust justamente para
 * quebrar essa circularidade "sem inventar credencial paralela" — a linha entra
 * na tabela `remote_devices` e a autenticação segue sendo a definitiva.
 *
 * `local.properties` **não é versionado** (já está no `.gitignore`), e os valores
 * só entram no `BuildConfig` da variante de depuração. No `release` os quatro
 * campos ficam vazios, e o código que os lê trata vazio como "não configurado".
 *
 * Como obter cada um, na máquina que roda o Lume:
 *
 *     lume.dev.token       = o mesmo valor exportado em LUME_REMOTE_DEV
 *     lume.dev.fingerprint = sha256sum <app_data_dir>/remote/identity.der
 *     lume.dev.host        = IP da máquina na rede do celular
 *     lume.dev.port        = 43140
 */
val propriedadesLocais = Properties().apply {
    val arquivo = rootProject.file("local.properties")
    if (arquivo.exists()) arquivo.inputStream().use { load(it) }
}

fun propriedadeLocal(chave: String): String = propriedadesLocais.getProperty(chave, "")

android {
    namespace = "lume.ai"
    compileSdk {
        version = release(36) {
            minorApiLevel = 1
        }
    }

    defaultConfig {
        applicationId = "lume.ai"

        // O piso é técnico, não comercial. `FontVariation` (a Inter variável, com
        // pesos 650/700/720/750) e o ícone adaptativo só existem a partir da 26.
        // Baixar daqui achata toda a tipografia para o peso 400 sem erro de build
        // — falha silenciosa, que é a pior espécie. Ver docs/ANDROID.md.
        minSdk = 26
        targetSdk = 36

        // Acompanha a versão do desktop; `versionCode` é monotônico e não deriva
        // do nome, para poder subir sem que o nome mude.
        versionCode = 1
        versionName = "0.5.4"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        // Declarados aqui, vazios, para que o código que os lê compile nas duas
        // variantes. Só o `debug` os preenche.
        buildConfigField("String", "DEV_HOST", "\"\"")
        buildConfigField("String", "DEV_PORT", "\"\"")
        buildConfigField("String", "DEV_TOKEN", "\"\"")
        buildConfigField("String", "DEV_FINGERPRINT", "\"\"")
    }

    buildTypes {
        debug {
            buildConfigField("String", "DEV_HOST", "\"${propriedadeLocal("lume.dev.host")}\"")
            buildConfigField("String", "DEV_PORT", "\"${propriedadeLocal("lume.dev.port")}\"")
            buildConfigField("String", "DEV_TOKEN", "\"${propriedadeLocal("lume.dev.token")}\"")
            buildConfigField(
                "String",
                "DEV_FINGERPRINT",
                "\"${propriedadeLocal("lume.dev.fingerprint")}\"",
            )
        }
        release {
            optimization {
                enable = false
            }
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
    buildFeatures {
        compose = true

        // A seção Demonstração de Ajustes só existe em debug, e quem decide isso
        // é `BuildConfig.DEBUG` — que não é gerado por padrão no AGP 9.
        buildConfig = true
    }
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.okhttp)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.hilt.android)
    implementation(libs.androidx.hilt.navigation.compose)
    ksp(libs.hilt.compiler)
    testImplementation(libs.junit)
    // `HeldCertificate` gera certificados autoassinados em teste — é o que permite
    // exercitar o caso negativo do pinning sem aparelho e sem servidor.
    testImplementation(libs.okhttp.tls)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(libs.androidx.junit)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
    debugImplementation(libs.androidx.compose.ui.tooling)
}
