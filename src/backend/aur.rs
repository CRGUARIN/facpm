use std::cmp::Reverse;
use std::time::Duration;

use anyhow::{Context, anyhow};
use chrono::{Local, TimeZone};

use crate::pkg::{Details, Package, Source};

const RPC_URL: &str = "https://aur.archlinux.org/rpc/";
const TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("facpm/", env!("CARGO_PKG_VERSION"));
pub(crate) const MAX_RESULTS: usize = 250;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AurPkg {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    #[serde(rename = "URL")]
    pub url: Option<String>,
    pub num_votes: u64,
    pub popularity: f64,
    pub maintainer: Option<String>,
    pub out_of_date: Option<i64>,
    pub first_submitted: i64,
    pub last_modified: i64,
    pub package_base: String,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub make_depends: Vec<String>,
    #[serde(default)]
    pub opt_depends: Vec<String>,
    #[serde(default)]
    pub license: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(serde::Deserialize)]
struct RpcResponse {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    results: Vec<AurPkg>,
}

impl AurPkg {
    pub fn to_package(&self, installed: Option<String>) -> Package {
        Package {
            name: self.name.clone(),
            version: self.version.clone(),
            description: self.description.clone().unwrap_or_default(),
            source: Source::Aur,
            installed,
        }
    }

    pub fn details(&self) -> Details {
        let mut rows = vec![
            row("Nombre", self.name.clone()),
            row("Versión", self.version.clone()),
            row("Descripción", or_dash(self.description.as_deref())),
            row("Repositorio", "aur".to_string()),
        ];
        if self.package_base != self.name {
            rows.push(row("Paquete base", self.package_base.clone()));
        }
        rows.extend([
            row("URL", or_dash(self.url.as_deref())),
            row("Licencias", join_list(&self.license, "  ")),
            row("Palabras clave", join_list(&self.keywords, "  ")),
            row("Depende de", join_list(&self.depends, "  ")),
            row(
                "Dependencias de compilación",
                join_list(&self.make_depends, "  "),
            ),
            row(
                "Dependencias opcionales",
                join_list(&self.opt_depends, "\n"),
            ),
            row(
                "Mantenedor",
                self.maintainer.as_deref().unwrap_or("huérfano").to_string(),
            ),
            row("Votos", self.num_votes.to_string()),
            row("Popularidad", format!("{:.2}", self.popularity)),
            row(
                "Desactualizado",
                self.out_of_date
                    .map_or_else(|| "No".to_string(), format_date),
            ),
            row("Enviado", format_date(self.first_submitted)),
            row("Última modificación", format_date(self.last_modified)),
            row(
                "URL del AUR",
                format!("https://aur.archlinux.org/packages/{}", self.name),
            ),
        ]);
        rows
    }
}

/// Pide al RPC solo el término más largo; el resto se filtra localmente.
pub fn search(query: &str) -> anyhow::Result<Vec<AurPkg>> {
    let lowered = query.to_lowercase();
    let terms: Vec<&str> = lowered.split_whitespace().collect();
    // `min_by_key` devuelve el primero en caso de empate: a igual largo, gana el primer término.
    let longest = terms
        .iter()
        .copied()
        .min_by_key(|term| Reverse(term.chars().count()))
        .filter(|term| term.chars().count() >= 2);
    let Some(longest) = longest else {
        return Ok(Vec::new());
    };

    let request = ureq::get(RPC_URL)
        .query("v", "5")
        .query("type", "search")
        .query("by", "name-desc")
        .query("arg", longest);
    let mut pkgs: Vec<AurPkg> = fetch(request)?
        .into_iter()
        .filter(|pkg| matches(pkg, &terms))
        .collect();
    sort_results(&mut pkgs, &terms.join(" "));
    Ok(pkgs)
}

/// Lo que usan la búsqueda y el orden; lo cumplen `AurPkg` y el índice local.
pub(crate) trait AurFields {
    fn name_str(&self) -> &str;
    fn description_str(&self) -> &str;
    fn popularity_value(&self) -> f64;
}

impl AurFields for AurPkg {
    fn name_str(&self) -> &str {
        &self.name
    }

    fn description_str(&self) -> &str {
        self.description.as_deref().unwrap_or_default()
    }

    fn popularity_value(&self) -> f64 {
        self.popularity
    }
}

impl<T: AurFields + ?Sized> AurFields for &T {
    fn name_str(&self) -> &str {
        (**self).name_str()
    }

    fn description_str(&self) -> &str {
        (**self).description_str()
    }

    fn popularity_value(&self) -> f64 {
        (**self).popularity_value()
    }
}

pub(crate) fn matches<P: AurFields, S: AsRef<str>>(pkg: &P, terms: &[S]) -> bool {
    terms.iter().all(|term| {
        let term = term.as_ref();
        contains_lc(pkg.name_str(), term) || contains_lc(pkg.description_str(), term)
    })
}

/// Primero el nombre igual a `query_lc`, luego popularidad y nombre; recorta a `MAX_RESULTS`.
pub(crate) fn sort_results<T: AurFields>(pkgs: &mut Vec<T>, query_lc: &str) {
    let cmp = |a: &T, b: &T| {
        (!eq_lc(a.name_str(), query_lc))
            .cmp(&!eq_lc(b.name_str(), query_lc))
            .then_with(|| b.popularity_value().total_cmp(&a.popularity_value()))
            .then_with(|| a.name_str().cmp(b.name_str()))
    };
    if pkgs.len() > MAX_RESULTS {
        // Con selección O(n) se separan los mejores y solo esos se ordenan.
        pkgs.select_nth_unstable_by(MAX_RESULTS - 1, cmp);
        pkgs.truncate(MAX_RESULTS);
    }
    pkgs.sort_by(cmp);
}

fn eq_lc(text: &str, query_lc: &str) -> bool {
    text.chars()
        .flat_map(char::to_lowercase)
        .eq(query_lc.chars())
}

/// Como `str::contains` sin distinguir mayúsculas y sin copiar el texto.
fn contains_lc(text: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if text.is_ascii() {
        return text
            .as_bytes()
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()));
    }
    text.char_indices()
        .any(|(i, _)| starts_with_lc(&text[i..], needle))
}

fn starts_with_lc(text: &str, needle: &str) -> bool {
    let mut lowered = text.chars().flat_map(char::to_lowercase);
    needle.chars().all(|n| lowered.next() == Some(n))
}

pub fn info(names: &[String]) -> anyhow::Result<Vec<AurPkg>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let mut request = ureq::get(RPC_URL).query("v", "5").query("type", "info");
    for name in names {
        request = request.query("arg[]", name);
    }
    fetch(request)
}

fn fetch(request: ureq::Request) -> anyhow::Result<Vec<AurPkg>> {
    let response: RpcResponse = request
        .timeout(TIMEOUT)
        .set("User-Agent", USER_AGENT)
        .call()?
        .into_json()
        .context("respuesta del AUR no válida")?;
    into_results(response)
}

fn into_results(response: RpcResponse) -> anyhow::Result<Vec<AurPkg>> {
    if response.kind == "error" {
        let message = response
            .error
            .unwrap_or_else(|| "error desconocido del AUR".to_string());
        return Err(anyhow!(message));
    }
    Ok(response.results)
}

fn row(key: &str, value: String) -> (String, String) {
    (key.to_string(), value)
}

fn or_dash(value: Option<&str>) -> String {
    value.filter(|v| !v.is_empty()).unwrap_or("—").to_string()
}

fn join_list(items: &[String], sep: &str) -> String {
    if items.is_empty() {
        "—".to_string()
    } else {
        items.join(sep)
    }
}

fn format_date(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_JSON: &str = r#"{"resultcount":1,"results":[{"Description":"Yet another yogurt. Pacman wrapper and AUR helper written in go.","FirstSubmitted":1480777574,"ID":2131229,"LastModified":1781904582,"Maintainer":null,"Name":"yay","NumVotes":370,"OutOfDate":null,"PackageBase":"yay","PackageBaseID":117489,"Popularity":4.044327,"URL":null,"URLPath":"/cgit/aur.git/snapshot/yay.tar.gz","Version":"13.0.1-1"}],"type":"search","version":5}"#;

    const INFO_JSON: &str = r#"{"resultcount":1,"results":[{"Description":"Demo package","FirstSubmitted":1480777574,"ID":1,"LastModified":1781904582,"Maintainer":"alguien","Name":"demo","NumVotes":3,"OutOfDate":null,"PackageBase":"demo-base","PackageBaseID":1,"Popularity":0.5,"URL":"https://example.org","URLPath":"/cgit/aur.git/snapshot/demo.tar.gz","Version":"1.0-1","Depends":["glibc"],"MakeDepends":["cargo"],"OptDepends":["alfa: primero","beta: segundo"],"License":["MIT"],"Keywords":["cli","tui"]}],"type":"multiinfo","version":5}"#;

    const ERROR_JSON: &str =
        r#"{"type":"error","error":"Too many package results.","resultcount":0,"results":[]}"#;

    fn parse(json: &str) -> RpcResponse {
        serde_json::from_str(json).expect("JSON de prueba válido")
    }

    fn first_pkg(json: &str) -> AurPkg {
        into_results(parse(json)).unwrap().remove(0)
    }

    fn value_of<'a>(details: &'a Details, key: &str) -> Option<&'a str> {
        details
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn pkg(name: &str, description: &str, popularity: f64) -> AurPkg {
        serde_json::from_value(serde_json::json!({
            "Name": name,
            "Version": "1.0-1",
            "Description": description,
            "NumVotes": 1,
            "Popularity": popularity,
            "FirstSubmitted": 0,
            "LastModified": 0,
            "PackageBase": name,
        }))
        .expect("paquete de prueba válido")
    }

    #[test]
    fn deserializes_search_result_and_converts_to_package() {
        let pkgs = into_results(parse(SEARCH_JSON)).unwrap();
        assert_eq!(pkgs.len(), 1);
        let yay = &pkgs[0];
        assert_eq!(yay.name, "yay");
        assert_eq!(yay.url, None);
        assert_eq!(yay.maintainer, None);
        assert!(yay.depends.is_empty());

        let pkg = yay.to_package(Some("13.0.0-1".to_string()));
        assert_eq!(pkg.source, Source::Aur);
        assert_eq!(pkg.version, "13.0.1-1");
        assert_eq!(pkg.installed.as_deref(), Some("13.0.0-1"));
    }

    #[test]
    fn error_response_becomes_err() {
        let err = into_results(parse(ERROR_JSON)).unwrap_err();
        assert_eq!(err.to_string(), "Too many package results.");
    }

    #[test]
    fn details_have_defaults_for_missing_fields() {
        let details = first_pkg(SEARCH_JSON).details();
        assert_eq!(value_of(&details, "Nombre"), Some("yay"));
        assert_eq!(value_of(&details, "Votos"), Some("370"));
        assert_eq!(value_of(&details, "Popularidad"), Some("4.04"));
        assert_eq!(
            value_of(&details, "URL del AUR"),
            Some("https://aur.archlinux.org/packages/yay")
        );
        assert_eq!(value_of(&details, "Mantenedor"), Some("huérfano"));
        assert_eq!(value_of(&details, "Desactualizado"), Some("No"));
        assert_eq!(value_of(&details, "URL"), Some("—"));
        assert_eq!(value_of(&details, "Depende de"), Some("—"));
        assert_eq!(value_of(&details, "Repositorio"), Some("aur"));
    }

    #[test]
    fn lists_are_joined_with_their_separator() {
        let details = first_pkg(INFO_JSON).details();
        assert_eq!(
            value_of(&details, "Dependencias opcionales"),
            Some("alfa: primero\nbeta: segundo")
        );
        assert_eq!(value_of(&details, "Depende de"), Some("glibc"));
        assert_eq!(value_of(&details, "Palabras clave"), Some("cli  tui"));
    }

    #[test]
    fn package_base_row_only_when_it_differs_from_name() {
        let same = first_pkg(SEARCH_JSON).details();
        assert_eq!(value_of(&same, "Paquete base"), None);

        let diff = first_pkg(INFO_JSON).details();
        let keys: Vec<&str> = diff.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys[3], "Repositorio");
        assert_eq!(keys[4], "Paquete base");
        assert_eq!(value_of(&diff, "Paquete base"), Some("demo-base"));
    }

    #[test]
    fn join_list_uses_two_spaces_or_dash_when_empty() {
        assert_eq!(join_list(&[], "  "), "—");
        assert_eq!(join_list(&["a".to_string(), "b".to_string()], "  "), "a  b");
    }

    #[test]
    fn format_date_has_minute_precision() {
        let s = format_date(1_700_000_000);
        assert_eq!(s.len(), 16, "formato %Y-%m-%d %H:%M, obtenido {s:?}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
    }

    #[test]
    fn short_or_empty_search_makes_no_request() {
        assert!(search("a").unwrap().is_empty());
        assert!(search("   ").unwrap().is_empty());
    }

    #[test]
    fn info_without_names_makes_no_request() {
        assert!(info(&[]).unwrap().is_empty());
    }

    #[test]
    fn exact_name_first_then_popularity_then_name() {
        let mut pkgs = vec![
            pkg("git-extras", "Utilidades para Git", 3.0),
            pkg("lazygit", "Interfaz de terminal para GIT", 2.0),
            pkg("git-lfs", "Almacenamiento de archivos grandes", 2.0),
            pkg("git", "Control de versiones", 0.5),
            pkg("firefox", "Navegador", 9.0),
        ];
        pkgs.retain(|p| matches(p, &["git"]));
        sort_results(&mut pkgs, "git");
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["git", "git-extras", "git-lfs", "lazygit"]);
    }

    #[test]
    fn matches_requires_every_term_case_insensitive() {
        let p = pkg("lazygit", "Interfaz de terminal para GIT", 2.0);
        assert!(matches(&p, &["git", "terminal"]));
        assert!(matches(&p, &["lazy", "interfaz"]));
        assert!(!matches(&p, &["git", "firefox"]));
    }

    #[test]
    fn matches_non_ascii_text() {
        let p = pkg("ñandu-tools", "Herramientas del NÚCLEO", 1.0);
        assert!(matches(&p, &["ñandu", "núcleo"]));
        assert!(!matches(&p, &["ñandú-x"]));
    }

    #[test]
    #[ignore = "red"]
    fn search_yay_online_is_sorted_by_popularity() {
        let pkgs = search("yay").expect("búsqueda en el AUR");
        assert!(pkgs.iter().any(|p| p.name == "yay"));
        assert!(pkgs.windows(2).all(|w| w[0].popularity >= w[1].popularity));
    }

    #[test]
    #[ignore = "red"]
    fn info_yay_online_has_depends() {
        let pkgs = info(&["yay".to_string()]).expect("info en el AUR");
        let yay = pkgs
            .iter()
            .find(|p| p.name == "yay")
            .expect("yay en la respuesta");
        assert!(!yay.depends.is_empty());
    }
}
