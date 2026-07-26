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

/**
 * Versão do aplicativo, derivada do `package.json` da raiz.
 *
 * Não é preferência de organização: `Lume-Mobile.apk` já foi publicado nas
 * versões 0.5.0 a 0.5.3, e o `versionCode` da 0.5.3 vale **5.003.000** pela
 * fórmula abaixo. O Android recusa instalar um APK cujo `versionCode` não seja
 * maior que o instalado, então qualquer número escolhido à mão aqui teria que
 * superar aquele — e um número escrito à mão que precisa superar outro número
 * escrito à mão em outro arquivo é um erro esperando a próxima release.
 *
 * A fórmula é a mesma já usada em campo, preservada dígito por dígito para que a
 * sequência publicada continue monotônica através da troca de implementação:
 *
 *     0.5.4 → 0×100000000 + 5×1000000 + 4×1000 + 0 = 5.004.000
 *
 * Os pesos impõem os limites `minor ≤ 99` e `patch ≤ 999`, folgados o bastante
 * para o esquema de versionamento deste projeto.
 *
 * O `--` de um sufixo como `0.6.0-rc1` é descartado (`^\d+` pega só os dígitos),
 * de modo que a candidata e a final compartilham o mesmo código. Isso é aceitável
 * porque candidatas não são publicadas como release; se um dia forem, o quarto
 * componente existe justamente para desempatá-las.
 */
val versaoDoProjeto: String = groovy.json.JsonSlurper()
    .parse(rootProject.file("../package.json"))
    .let { it as Map<*, *> }
    .let { it["version"] as String }

val codigoDaVersao: Int = versaoDoProjeto
    .split(".")
    .map { parte -> Regex("^\\d+").find(parte)?.value?.toInt() ?: 0 }
    .let { partes -> partes + List(maxOf(0, 4 - partes.size)) { 0 } }
    .also { partes ->
        // Fora destas faixas os pesos se sobrepõem e dois nomes de versão
        // diferentes produzem o mesmo código — `0.100.0` colidiria com `1.0.0`.
        // O Android então recusaria a atualização como "mesma versão", em silêncio
        // e só nos aparelhos. Parar o build aqui é a única forma de esse limite
        // aparecer para quem pode corrigi-lo.
        require(partes[1] <= 99 && partes[2] <= 999 && partes[3] <= 999) {
            "versionCode não representa $versaoDoProjeto sem colisão: " +
                "minor deve ser ≤ 99 e patch ≤ 999. Ajuste a fórmula antes de publicar."
        }
    }
    .let { (it[0] * 100_000_000) + (it[1] * 1_000_000) + (it[2] * 1_000) + it[3] }

android {
    // Herdado da implementação anterior deste aplicativo, e **imutável**: quatro
    // APKs já foram publicados sob este identificador. Trocá-lo faria o Android
    // tratar esta versão como um aplicativo diferente, instalado ao lado do que a
    // pessoa já tem, em vez de uma atualização dele.
    namespace = "com.tulerws.lume.mobile"
    compileSdk {
        version = release(36) {
            minorApiLevel = 1
        }
    }

    defaultConfig {
        applicationId = "com.tulerws.lume.mobile"

        // O piso é técnico, não comercial. `FontVariation` (a Inter variável, com
        // pesos 650/700/720/750) e o ícone adaptativo só existem a partir da 26.
        // Baixar daqui achata toda a tipografia para o peso 400 sem erro de build
        // — falha silenciosa, que é a pior espécie. Ver docs/ANDROID.md.
        minSdk = 26
        targetSdk = 36

        versionCode = codigoDaVersao
        versionName = versaoDoProjeto

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        // Declarados aqui, vazios, para que o código que os lê compile nas duas
        // variantes. Só o `debug` os preenche.
        buildConfigField("String", "DEV_HOST", "\"\"")
        buildConfigField("String", "DEV_PORT", "\"\"")
        buildConfigField("String", "DEV_TOKEN", "\"\"")
        buildConfigField("String", "DEV_FINGERPRINT", "\"\"")
    }

    /**
     * Assinatura de release, lida **do ambiente**.
     *
     * Todo APK precisa ser assinado, e não há autoridade certificadora nisso: a
     * chave é gerada por nós e ninguém a atesta. Ela não serve para provar
     * identidade, serve para provar **continuidade** — o Android guarda a chave
     * que instalou o aplicativo e recusa atualização assinada por outra. É o
     * mesmo raciocínio do certificado do desktop: a chave é a identidade.
     *
     * A consequência é dura e vale estar escrita aqui: **perder a chave é perder
     * o direito de atualizar.** Quem já instalou precisaria desinstalar.
     *
     * Por isso ela não mora nesta máquina nem neste repositório. Os quatro
     * valores vêm de variáveis de ambiente, preenchidas pelos *secrets* do
     * GitHub Actions no único lugar que assina para valer, que é o CI.
     */
    val keystoreDoAmbiente = System.getenv("ANDROID_KEYSTORE_PATH")
        ?.takeIf { it.isNotBlank() }
        ?.let(::file)
        ?.takeIf { it.exists() }

    signingConfigs {
        if (keystoreDoAmbiente != null) {
            create("release") {
                storeFile = keystoreDoAmbiente
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
                // v1 assina arquivo a arquivo dentro do zip e só é necessário
                // abaixo da API 24. O piso aqui é 26, então ele é peso morto.
                enableV1Signing = false
                enableV2Signing = true
                enableV3Signing = true
            }
        }
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
            // Sem keystore no ambiente, o `release` cai para a chave de
            // depuração. Isso é deliberado: permite `assembleRelease` nesta
            // máquina, para testar o artefato, sem a chave de produção existir
            // aqui.
            //
            // **E é perigoso**, porque um APK assim é publicável sem ninguém
            // notar. O guarda-corpo não está neste arquivo e sim no
            // `installers.yml`, que falha explicitamente quando o segredo do
            // keystore não está presente.
            signingConfig = signingConfigs.findByName("release")
                ?: signingConfigs.getByName("debug")

            // R8 desligado neste primeiro release, por decisão registrada: o
            // artefato publicado é o mesmo que foi testado no aparelho. Ligar o
            // encolhimento introduz uma classe de falha que só aparece em
            // release, e ela merece ser provada sozinha, depois.
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

        // `BuildConfig` não é gerado por padrão no AGP 9, e aqui ele carrega o
        // `versionName` que a tela Sobre mostra, mais os quatro campos `DEV_*`
        // lidos de `local.properties`.
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
    implementation(libs.androidx.camera.camera2)
    implementation(libs.androidx.camera.lifecycle)
    implementation(libs.androidx.camera.view)
    implementation(libs.mlkit.barcode.scanning)
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.okhttp)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.hilt.android)
    implementation(libs.androidx.hilt.navigation.compose)
    ksp(libs.hilt.compiler)
    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
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
