//! Montagem de queries e campos da Drive API (SEG-21, desempenho via `fields=`).

pub const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
/// Prefixo dos tipos nativos (Docs, Sheets…) — sem conteúdo binário nem checksum (RN-34).
pub const GOOGLE_APPS_MIME_PREFIX: &str = "application/vnd.google-apps.";

/// RN-02: marca privada (só este app enxerga) da pasta raiz.
pub const ROOT_PROPERTY_KEY: &str = "syncly";
pub const ROOT_PROPERTY_VALUE: &str = "root";
pub const ROOT_FOLDER_NAME: &str = "Syncly";

/// Só os campos usados — respostas menores e mais rápidas.
pub const FILE_FIELDS: &str =
    "id,name,mimeType,size,md5Checksum,modifiedTime,lastModifyingUser(displayName),trashed";
pub const LIST_FIELDS: &str = "nextPageToken,files(id,name,mimeType,size,md5Checksum,modifiedTime,lastModifyingUser(displayName),trashed)";
pub const PAGE_SIZE: u32 = 1000;

/// SEG-21: escapa um valor para uso entre aspas simples numa query do Drive.
pub fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

pub fn children_of(parent_id: &str) -> String {
    format!("'{}' in parents and trashed = false", escape(parent_id))
}

pub fn child_named(parent_id: &str, name: &str) -> String {
    format!(
        "'{}' in parents and name = '{}' and trashed = false",
        escape(parent_id),
        escape(name)
    )
}

pub fn app_root() -> String {
    format!(
        "appProperties has {{ key='{ROOT_PROPERTY_KEY}' and value='{ROOT_PROPERTY_VALUE}' }} \
         and mimeType = '{FOLDER_MIME}' and trashed = false"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seg21_aspas_nao_quebram_a_query() {
        let q = child_named("pai", "x' or name contains 'a");
        assert_eq!(
            q,
            r"'pai' in parents and name = 'x\' or name contains \'a' and trashed = false"
        );
    }

    #[test]
    fn seg21_barra_invertida_e_escapada_primeiro() {
        assert_eq!(escape(r"a\'b"), r"a\\\'b");
    }

    #[test]
    fn query_da_raiz() {
        assert!(app_root().contains("appProperties has { key='syncly' and value='root' }"));
    }
}
