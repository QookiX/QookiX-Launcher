use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use std::{error::Error, fs, io::Read, path::Path};

fn decode(encoded: &str) -> Result<String, Box<dyn Error>> {
    Ok(String::from_utf8(STANDARD.decode(encoded.trim())?)?)
}

fn verify_bundle(key: &PublicKey, file: &Path) -> Result<(), Box<dyn Error>> {
    let mut signature_path = file.as_os_str().to_os_string();
    signature_path.push(".sig");
    let signature = Signature::decode(&decode(&fs::read_to_string(signature_path)?)?)?;
    let mut file = fs::File::open(file)?;
    let mut stream = key.verify_stream(&signature)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        stream.update(&buffer[..size]);
    }
    stream.finalize()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let public_key = arguments.next().ok_or("missing Tauri updater public key")?;
    let key = PublicKey::decode(&decode(
        public_key.to_str().ok_or("invalid updater public key")?,
    )?)?;
    for artifact in arguments {
        let path = Path::new(&artifact);
        verify_bundle(&key, path)
            .map_err(|error| format!("{}: invalid updater signature: {error}", path.display()))?;
        println!("Verified updater signature for {}", path.display());
    }
    Ok(())
}
