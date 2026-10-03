//! Caminhos relativos validados (RN-16, SEG-20). Usados no destino das tarefas,
//! nos IDs do `LocalProvider` e em qualquer caminho vindo de fora.

use std::fmt;

use serde::{Deserialize, Serialize};

pub const MAX_NAME_LEN: usize = 255;
pub const MAX_DEPTH: usize = 32;

/// Valida um único nome de arquivo/pasta (SEG-20).
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("nome vazio".into());
    }
    if name == "." || name == ".." {
        return Err("'.' e '..' não são nomes permitidos".into());
    }
    if name.contains('/') {
        return Err("o nome não pode conter '/'".into());
    }
    if name.chars().any(char::is_control) {
        return Err("o nome não pode conter caracteres de controle".into());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("o nome excede {MAX_NAME_LEN} bytes"));
    }
    Ok(())
}

/// Caminho relativo normalizado, componentes separados por `/`. Vazio = raiz.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelPath(String);

impl RelPath {
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Aceita barras nas pontas (`/a/b/` → `a/b`); rejeita `.`, `..`, vazios e controles.
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(Self::root());
        }
        let mut depth = 0;
        for part in trimmed.split('/') {
            validate_name(part).map_err(|e| format!("caminho '{input}': {e}"))?;
            depth += 1;
        }
        if depth > MAX_DEPTH {
            return Err(format!("caminho com mais de {MAX_DEPTH} níveis"));
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|c| !c.is_empty())
    }

    pub fn depth(&self) -> usize {
        self.components().count()
    }

    pub fn join(&self, name: &str) -> Result<Self, String> {
        validate_name(name)?;
        Ok(Self(join_str(&self.0, name)))
    }

    /// `true` se `self` é igual a `other` ou está dentro dele (por componentes).
    pub fn is_within(&self, other: &RelPath) -> bool {
        is_within_str(&self.0, &other.0)
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for RelPath {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<RelPath> for String {
    fn from(value: RelPath) -> Self {
        value.0
    }
}

/// Junta dois caminhos relativos em formato string (`""` é a raiz).
pub fn join_str(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

/// `path` é igual a `ancestor` ou descendente dele? Comparação por componentes.
pub fn is_within_str(path: &str, ancestor: &str) -> bool {
    ancestor.is_empty()
        || path == ancestor
        || (path.len() > ancestor.len()
            && path.starts_with(ancestor)
            && path.as_bytes()[ancestor.len()] == b'/')
}

/// Pai de um caminho relativo em string (`"a/b"` → `"a"`, `"a"` → `""`).
pub fn parent_str(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_barras_nas_pontas() {
        assert_eq!(RelPath::parse("/a/b/").unwrap().as_str(), "a/b");
        assert!(RelPath::parse("").unwrap().is_root());
    }

    #[test]
    fn rejeita_travessia_e_controles() {
        for bad in ["a/../b", "..", "a/./b", "a//b", "a/\nb", "a/\0"] {
            assert!(
                RelPath::parse(bad).is_err(),
                "{bad:?} deveria ser rejeitado"
            );
        }
    }

    #[test]
    fn limita_profundidade() {
        let deep = vec!["x"; MAX_DEPTH + 1].join("/");
        assert!(RelPath::parse(&deep).is_err());
    }

    #[test]
    fn contencao_por_componentes() {
        let a = RelPath::parse("Docs").unwrap();
        assert!(RelPath::parse("Docs/x").unwrap().is_within(&a));
        assert!(a.is_within(&a));
        assert!(!RelPath::parse("Docs2").unwrap().is_within(&a));
        assert!(a.is_within(&RelPath::root()));
    }

    #[test]
    fn pai() {
        assert_eq!(parent_str("a/b/c"), "a/b");
        assert_eq!(parent_str("a"), "");
    }
}
