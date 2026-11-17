use anyhow::Result;

pub(crate) fn decode_private_key(
    text: &str,
    passphrase: Option<&str>,
) -> Result<russh::keys::PrivateKey> {
    russh::keys::decode_secret_key(text.trim(), passphrase.filter(|value| !value.is_empty()))
        .map_err(|_| anyhow::anyhow!("Invalid SSH private key or incorrect key passphrase"))
}
