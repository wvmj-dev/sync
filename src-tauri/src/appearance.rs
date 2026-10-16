//! Aparência do sistema: claro/escuro e cor de destaque, lidos do portal do desktop
//! (`org.freedesktop.appearance`), que GNOME, KDE e outros publicam. Quando o tema muda, a
//! janela recebe o evento `appearance-changed`. Sem portal, a interface segue o
//! `prefers-color-scheme` do navegador embutido.

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use zbus::zvariant::{OwnedValue, Value};

const NAMESPACE: &str = "org.freedesktop.appearance";

/// O que a interface precisa saber do tema do sistema.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    /// `"light"`, `"dark"` ou `None` (sem portal: a interface usa `prefers-color-scheme`).
    pub scheme: Option<&'static str>,
    /// Cor de destaque em `#rrggbb`, se o sistema informar.
    pub accent: Option<String>,
}

/// `color-scheme` do portal: 1 = prefere escuro, 2 = prefere claro, 0 = sem preferência.
/// "Sem preferência" é o modo **claro** do GNOME (estilo `default`) — vale claro, como nos apps
/// GNOME (libadwaita). Deixar para o navegador embutido não serve: ele decide pelo tema GTK,
/// que pode continuar escuro (ex.: Orchis-Dark) com o sistema no modo claro.
fn parse_scheme(value: u32) -> Option<&'static str> {
    match value {
        1 => Some("dark"),
        0 | 2 => Some("light"),
        _ => None,
    }
}

/// `accent-color` do portal: (r, g, b) entre 0 e 1; fora disso = não definida.
fn parse_accent((r, g, b): (f64, f64, f64)) -> Option<String> {
    let channel = |c: f64| (0.0..=1.0).contains(&c).then(|| (c * 255.0).round() as u8);
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        channel(r)?,
        channel(g)?,
        channel(b)?
    ))
}

async fn proxy() -> zbus::Result<zbus::Proxy<'static>> {
    let conn = zbus::Connection::session().await?;
    zbus::Proxy::new(
        &conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )
    .await
}

/// Lê uma chave; portais antigos só têm `Read`, que embrulha o valor em outra variante.
async fn read(proxy: &zbus::Proxy<'_>, key: &str) -> Option<OwnedValue> {
    if let Ok(value) = proxy
        .call::<_, _, OwnedValue>("ReadOne", &(NAMESPACE, key))
        .await
    {
        return Some(value);
    }
    let wrapped: OwnedValue = proxy.call("Read", &(NAMESPACE, key)).await.ok()?;
    match &*wrapped {
        Value::Value(inner) => OwnedValue::try_from(&**inner).ok(),
        _ => Some(wrapped),
    }
}

async fn read_all(proxy: &zbus::Proxy<'_>) -> Appearance {
    let scheme = read(proxy, "color-scheme")
        .await
        .and_then(|v| u32::try_from(v).ok())
        .and_then(parse_scheme);
    let accent = read(proxy, "accent-color")
        .await
        .and_then(|v| <(f64, f64, f64)>::try_from(v).ok())
        .and_then(parse_accent);
    Appearance { scheme, accent }
}

/// Tema atual do sistema; sem portal, vazio (a interface usa o padrão do navegador).
pub async fn current() -> Appearance {
    match proxy().await {
        Ok(proxy) => read_all(&proxy).await,
        Err(e) => {
            tracing::debug!("portal de aparência indisponível: {e}");
            Appearance::default()
        }
    }
}

/// Avisa a janela (`appearance-changed`) sempre que o tema do sistema mudar.
pub async fn watch(app: AppHandle) {
    let result: zbus::Result<()> = async {
        let proxy = proxy().await?;
        let mut changes = proxy.receive_signal("SettingChanged").await?;
        while let Some(message) = changes.next().await {
            let Ok((namespace, key, _)) =
                message.body().deserialize::<(String, String, OwnedValue)>()
            else {
                continue;
            };
            if namespace == NAMESPACE && (key == "color-scheme" || key == "accent-color") {
                let appearance = read_all(&proxy).await;
                tracing::info!(
                    scheme = appearance.scheme.unwrap_or("—"),
                    accent = appearance.accent.as_deref().unwrap_or("—"),
                    "tema do sistema mudou"
                );
                let _ = app.emit("appearance-changed", appearance);
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!("mudanças de tema não serão acompanhadas: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_o_esquema_de_cores_do_portal() {
        assert_eq!(parse_scheme(1), Some("dark"));
        assert_eq!(parse_scheme(2), Some("light"));
        // Modo claro do GNOME ("default") chega como "sem preferência".
        assert_eq!(parse_scheme(0), Some("light"));
        assert_eq!(parse_scheme(7), None);
    }

    #[test]
    fn le_a_cor_de_destaque_do_portal() {
        // Azul do GNOME, como o portal informa.
        assert_eq!(
            parse_accent((0.207843, 0.517647, 0.894118)).as_deref(),
            Some("#3584e4")
        );
        // Fora de 0..1 = sem cor definida.
        assert_eq!(parse_accent((-1.0, 0.5, 0.5)), None);
        assert_eq!(parse_accent((0.5, 1.5, 0.5)), None);
    }
}
