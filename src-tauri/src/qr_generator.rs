//! Geração de QR Code para exibição na tela do desktop.
//!
//! O módulo não sabe o que é pareamento nem o que é o Lume: recebe texto,
//! devolve matriz e SVG. Quem monta a URI `lume://pair` é o pareamento.
//!
//! Duas convenções que o resto do sistema depende e que os testes fixam:
//!
//! - a matriz é *row-major* com origem no canto superior esquerdo, e `is_dark`
//!   recebe `(x, y)` nessa ordem — trocar isso transpõe o código e nenhum
//!   leitor o reconhece;
//! - o SVG é **sempre** escuro sobre claro, com zona de silêncio embutida,
//!   independentemente do tema da interface.

use fast_qr::{QRBuilder, ECL};

/// Zona de silêncio em módulos. Quatro é o mínimo que a norma exige; abaixo
/// disso muitos leitores simplesmente não encontram o código.
pub const QUIET_ZONE: usize = 4;

/// Correção de erro média, 15%. Nível mais alto adensaria a matriz e deixaria
/// cada módulo menor na tela, que é o oposto do que ajuda uma câmera de celular
/// a ler um monitor a trinta centímetros.
const ERROR_CORRECTION: ECL = ECL::M;

/// Cores fixas, deliberadamente fora do sistema de tema.
///
/// Um QR claro sobre fundo escuro é recusado por boa parte dos leitores de
/// Android, que assumem polaridade normal. O modo escuro do Lume **não** pode
/// inverter isto, e é por isso que o fundo vem dentro do próprio SVG em vez de
/// ser herdado do painel.
const LIGHT: &str = "#F9FBFA";
const DARK: &str = "#17201D";

/// Matriz de módulos, sem zona de silêncio.
pub struct QrMatrix {
    size: usize,
    modules: Vec<bool>,
    version: i16,
}

/// Compacto de propósito: derivar imprimiria a matriz inteira em cada falha de
/// teste, e o que interessa ao diagnosticar é tamanho e versão.
impl std::fmt::Debug for QrMatrix {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "QrMatrix {{ versão {}, {}×{} módulos }}",
            self.version, self.size, self.size
        )
    }
}

impl QrMatrix {
    /// Lado da matriz em módulos, sem contar a zona de silêncio.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Versão do QR (1 a 40). Serve para diagnóstico: versão subindo é sinal de
    /// que a URI cresceu, e URI grande vira código denso e difícil de ler.
    pub fn version(&self) -> i16 {
        self.version
    }

    /// `(x, y)` com origem no canto superior esquerdo. Fora dos limites é
    /// claro, o que faz a zona de silêncio cair naturalmente.
    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        if x >= self.size || y >= self.size {
            return false;
        }
        self.modules[y * self.size + x]
    }
}

/// Codifica o texto.
///
/// O conteúdo é tratado como bytes, então o modo escolhido é o de 8 bits. Não
/// adianta tentar caber no modo alfanumérico, mais denso: ele só aceita
/// maiúsculas e um punhado de símbolos, e a URI de pareamento carrega base64url,
/// que distingue maiúscula de minúscula.
pub fn encode(text: &str) -> Result<QrMatrix, String> {
    if text.is_empty() {
        return Err("Não há conteúdo para gerar o QR Code".to_string());
    }
    let code = QRBuilder::new(text.as_bytes())
        .ecl(ERROR_CORRECTION)
        .build()
        .map_err(|error| match error {
            fast_qr::qr::QRCodeError::EncodedData => {
                "O conteúdo é longo demais para um QR Code".to_string()
            }
            other => format!("Não foi possível gerar o QR Code: {other}"),
        })?;

    let size = code.size;
    // O `fast_qr` guarda os módulos num vetor de tamanho fixo, dimensionado
    // para a maior versão possível. Só a região `size × size` do início vale.
    let modules = code.data[..size * size]
        .iter()
        .map(|module| module.value())
        .collect();

    Ok(QrMatrix {
        size,
        modules,
        // Versão derivada do lado: a norma define lado = versão × 4 + 17.
        version: ((size as i16) - 17) / 4,
    })
}

/// Desenha a matriz como SVG, com a zona de silêncio já incluída.
///
/// O `viewBox` é medido em módulos, não em pixels: quem exibe escolhe o tamanho
/// final por CSS sem que nada precise ser gerado de novo. O `crispEdges` impede
/// que a suavização do navegador borre a borda entre módulos vizinhos, que é
/// onde um leitor começa a errar.
pub fn to_svg(matrix: &QrMatrix) -> String {
    let side = matrix.size() + QUIET_ZONE * 2;
    let path = dark_path(matrix);

    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {side} {side}" "#,
            r#"shape-rendering="crispEdges" role="img" aria-label="QR Code de pareamento">"#,
            r#"<rect width="{side}" height="{side}" fill="{light}"/>"#,
            r#"<path fill="{dark}" d="{path}"/>"#,
            "</svg>"
        ),
        side = side,
        light = LIGHT,
        dark = DARK,
        path = path
    )
}

/// Um caminho único com todos os módulos escuros, agrupando sequências
/// horizontais.
///
/// Um `<rect>` por módulo produziria milhares de elementos para uma URI de
/// pareamento; agrupando as sequências, o SVG cabe em alguns kilobytes e o
/// webview desenha sem engasgar.
fn dark_path(matrix: &QrMatrix) -> String {
    let mut path = String::new();
    for y in 0..matrix.size() {
        let mut x = 0;
        while x < matrix.size() {
            if !matrix.is_dark(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < matrix.size() && matrix.is_dark(x, y) {
                x += 1;
            }
            let length = x - start;
            path.push_str(&format!(
                "M{} {}h{length}v1h-{length}z",
                start + QUIET_ZONE,
                y + QUIET_ZONE
            ));
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAIRING_URI: &str = concat!(
        "lume://pair?v=1&f=Zm9vYmFyYmF6cXV1eGZvb2JhcmJhenF1dXhmb28&",
        "c=YmFycXV1eGZvb2JhcmJhenF1dXhmb29iYXJiYXo&p=43140&",
        "h=192.168.0.14,10.0.0.7,fe80::1&n=lume-desktop"
    );

    /// Rasteriza a matriz como um leitor a veria: módulos ampliados, zona de
    /// silêncio ao redor, sem suavização.
    fn bitmap(matrix: &QrMatrix, scale: usize) -> (usize, Vec<bool>) {
        let side = (matrix.size() + QUIET_ZONE * 2) * scale;
        let mut pixels = vec![false; side * side];
        for y in 0..matrix.size() {
            for x in 0..matrix.size() {
                if !matrix.is_dark(x, y) {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let py = (y + QUIET_ZONE) * scale + dy;
                        let px = (x + QUIET_ZONE) * scale + dx;
                        pixels[py * side + px] = true;
                    }
                }
            }
        }
        (side, pixels)
    }

    fn decode(matrix: &QrMatrix) -> String {
        let (side, pixels) = bitmap(matrix, 6);
        let mut image =
            rqrr::PreparedImage::prepare_from_bitmap(side, side, |x, y| pixels[y * side + x]);
        let grids = image.detect_grids();
        assert_eq!(grids.len(), 1, "o leitor precisa encontrar exatamente um QR");
        let (_, content) = grids[0].decode().expect("decodificação");
        content
    }

    /// O teste que justifica a dependência de desenvolvimento: um leitor
    /// independente do codificador recupera exatamente o texto original.
    ///
    /// É o que prova a orientação da matriz. Uma transposição passaria por
    /// qualquer verificação estrutural — a matriz continuaria com aparência de
    /// QR — e só falharia na mão do usuário, apontando a câmera.
    #[test]
    fn an_independent_reader_recovers_the_original_text() {
        let matrix = encode(PAIRING_URI).expect("codifica");
        assert_eq!(decode(&matrix), PAIRING_URI);
    }

    /// Orçamento de densidade, medido e fixado.
    ///
    /// A densidade do QR é função direta do tamanho da URI, e QR denso é QR que
    /// a câmera erra: no painel do Lume, 392 px de largura, um código de versão
    /// 9 deixa cada módulo com menos de quatro pixels. As fronteiras abaixo são
    /// o que o pareamento tem de orçamento — e o teste denuncia se alguém mudar
    /// o nível de correção de erro sem perceber o efeito no tamanho.
    #[test]
    fn the_density_budget_is_explicit() {
        assert_eq!(encode(&"a".repeat(106)).expect("codifica").version(), 6);
        assert_eq!(encode(&"a".repeat(107)).expect("codifica").version(), 7);
        assert_eq!(encode(&"a".repeat(122)).expect("codifica").version(), 7);
        assert_eq!(encode(&"a".repeat(123)).expect("codifica").version(), 8);
        assert_eq!(encode(&"a".repeat(152)).expect("codifica").version(), 8);
        assert_eq!(encode(&"a".repeat(153)).expect("codifica").version(), 9);
    }

    #[test]
    fn the_finder_pattern_sits_at_the_origin() {
        let matrix = encode(PAIRING_URI).expect("codifica");
        // Anel externo escuro, anel interno claro, miolo escuro.
        assert!(matrix.is_dark(0, 0));
        assert!(matrix.is_dark(6, 0));
        assert!(!matrix.is_dark(1, 1));
        assert!(!matrix.is_dark(5, 5));
        assert!(matrix.is_dark(3, 3));
    }

    #[test]
    fn out_of_bounds_reads_as_light_so_the_quiet_zone_falls_out_naturally() {
        let matrix = encode(PAIRING_URI).expect("codifica");
        let size = matrix.size();
        assert!(!matrix.is_dark(size, 0));
        assert!(!matrix.is_dark(0, size));
        assert!(!matrix.is_dark(usize::MAX, usize::MAX));
    }

    /// Lê de volta o caminho que `dark_path` escreveu, no formato exato dele:
    /// `M{x} {y}h{n}v1h-{n}z`. Devolve o lado do `viewBox` e as sequências.
    fn parse_svg(svg: &str) -> (usize, Vec<(usize, usize, usize)>) {
        let side = svg
            .split(r#"viewBox="0 0 "#)
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|value| value.parse().ok())
            .expect("viewBox");
        let path = svg
            .split(r#" d=""#)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("caminho");

        let runs = path
            .split('M')
            .skip(1)
            .map(|segment| {
                let (coordinates, rest) = segment.split_once('h').expect("traço horizontal");
                let (x, y) = coordinates.split_once(' ').expect("par de coordenadas");
                let length = rest.split('v').next().expect("comprimento");
                (
                    x.parse().expect("x"),
                    y.parse().expect("y"),
                    length.parse().expect("comprimento"),
                )
            })
            .collect();
        (side, runs)
    }

    /// O teste que fecha a lacuna entre "a matriz está certa" e "o desenho está
    /// certo".
    ///
    /// Os outros testes vão da matriz direto ao leitor, pulando o desenhador.
    /// Uma troca de eixos no `M{x} {y}`, ou um comprimento de sequência com um
    /// a menos, produziria um SVG com aparência de QR que nenhuma câmera lê — e
    /// passaria em tudo o que existe aqui. Este parte do **SVG entregue**,
    /// rasteriza e manda decodificar.
    #[test]
    fn the_rendered_svg_decodes_back_to_the_original_text() {
        let svg = to_svg(&encode(PAIRING_URI).expect("codifica"));
        let (side, runs) = parse_svg(&svg);

        let scale = 6;
        let pixels_side = side * scale;
        let mut pixels = vec![false; pixels_side * pixels_side];
        for (x, y, length) in runs {
            for dy in 0..scale {
                for dx in 0..(length * scale) {
                    let py = y * scale + dy;
                    let px = x * scale + dx;
                    pixels[py * pixels_side + px] = true;
                }
            }
        }

        let mut image = rqrr::PreparedImage::prepare_from_bitmap(pixels_side, pixels_side, |x, y| {
            pixels[y * pixels_side + x]
        });
        let grids = image.detect_grids();
        assert_eq!(grids.len(), 1, "o SVG desenhado precisa conter um QR legível");
        assert_eq!(grids[0].decode().expect("decodificação").1, PAIRING_URI);
    }

    #[test]
    fn the_svg_carries_its_own_quiet_zone() {
        let matrix = encode(PAIRING_URI).expect("codifica");
        let side = matrix.size() + QUIET_ZONE * 2;
        let svg = to_svg(&matrix);

        assert!(svg.contains(&format!(r#"viewBox="0 0 {side} {side}""#)));
        // Nenhum módulo escuro pode encostar na borda: o primeiro traço começa
        // em quatro módulos para dentro, nos dois eixos.
        assert!(svg.contains(&format!("M{QUIET_ZONE} {QUIET_ZONE}h")));
        for coordinate in 0..QUIET_ZONE {
            assert!(
                !svg.contains(&format!("M{coordinate} ")),
                "traço iniciando dentro da zona de silêncio, em x={coordinate}"
            );
        }
    }

    /// O modo escuro não pode inverter o QR. Este teste é o que impede alguém
    /// de trocar as cores por tokens de tema mais tarde sem perceber o efeito.
    #[test]
    fn the_svg_is_always_dark_on_light() {
        let svg = to_svg(&encode(PAIRING_URI).expect("codifica"));
        assert!(svg.contains(&format!(r#"<rect width="#)));
        assert!(svg.contains(&format!(r#"fill="{LIGHT}""#)));
        assert!(svg.contains(&format!(r#"<path fill="{DARK}""#)));
        assert!(
            !svg.contains("currentColor"),
            "a cor não pode ser herdada do painel"
        );
    }

    #[test]
    fn the_svg_scales_by_css_instead_of_being_regenerated() {
        let svg = to_svg(&encode(PAIRING_URI).expect("codifica"));
        assert!(!svg.contains("width=\"0 "));
        assert!(
            !svg.contains(" width=\"") || svg.matches(" width=\"").count() == 1,
            "só o retângulo de fundo tem largura; o <svg> vive do viewBox"
        );
        assert!(svg.contains(r#"shape-rendering="crispEdges""#));
    }

    #[test]
    fn empty_text_is_refused_instead_of_producing_a_useless_code() {
        assert!(encode("").is_err());
    }

    #[test]
    fn oversized_content_fails_with_a_readable_message() {
        let error = encode(&"a".repeat(4096)).expect_err("conteúdo longo demais");
        assert!(error.contains("longo demais"), "mensagem opaca: {error}");
    }

    /// A versão do QR sobe conforme a URI cresce. O pareamento vai precisar
    /// limitar a lista de endereços em `h=`; este teste é o lembrete de que o
    /// custo existe e é mensurável.
    #[test]
    fn version_grows_with_the_payload() {
        let short = encode("lume://pair?v=1").expect("codifica");
        let long = encode(&format!("{PAIRING_URI}{}", ",10.1.2.3".repeat(20)))
            .expect("codifica");
        assert!(
            long.version() > short.version(),
            "curto v{}, longo v{}",
            short.version(),
            long.version()
        );
    }
}
