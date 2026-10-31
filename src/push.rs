use base64ct::{Base64UrlUnpadded, Encoding};
use postgres::Client;
use serde::Deserialize;
use web_push_native::{
    jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair},
    p256::PublicKey,
    Auth, WebPushBuilder,
};

const CONTACT: &str = "effiongdestiny166@gmail.com";

#[derive(Clone)]
pub struct Sub {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

// what the browser gives us after the person taps allow
#[derive(Deserialize)]
pub struct Keys {
    pub p256dh: String,
    pub auth: String,
}

pub struct Vapid {
    pub pair: ES256KeyPair,
    pub public: String, // base64url, what the browser needs to subscribe
}

pub enum Outcome {
    Sent,
    Gone,   // the phone unsubscribed or the address is dead, forget it
    Failed, // temporary problem, keep it
}

pub fn vapid(db: &mut Client) -> Vapid {
    db.batch_execute("CREATE TABLE IF NOT EXISTS settings (k TEXT PRIMARY KEY, v TEXT NOT NULL);")
        .expect("settings table");
    let saved: Option<String> = db
        .query_opt("SELECT v FROM settings WHERE k = 'vapid_private'", &[])
        .ok()
        .flatten()
        .map(|r| r.get(0));
    let pair = saved
        .and_then(|s| Base64UrlUnpadded::decode_vec(&s).ok())
        .and_then(|b| ES256KeyPair::from_bytes(&b).ok())
        .unwrap_or_else(|| {
            let pair = ES256KeyPair::generate();
            let _ = db.execute(
                "INSERT INTO settings (k, v) VALUES ('vapid_private', $1) ON CONFLICT (k) DO UPDATE SET v = $1",
                &[&Base64UrlUnpadded::encode_string(&pair.to_bytes())],
            );
            pair
        });
    let public = Base64UrlUnpadded::encode_string(&pair.public_key().public_key().to_bytes_uncompressed());
    Vapid { pair, public }
}

pub fn send(pair: &ES256KeyPair, agent: &ureq::Agent, sub: &Sub, payload: &str) -> Outcome {
    let Ok(uri) = sub.endpoint.parse::<http::Uri>() else {
        return Outcome::Gone;
    };
    let Some(key) = Base64UrlUnpadded::decode_vec(&sub.p256dh)
        .ok()
        .and_then(|b| PublicKey::from_sec1_bytes(&b).ok())
    else {
        return Outcome::Gone;
    };
    let Some(auth) = Base64UrlUnpadded::decode_vec(&sub.auth).ok().filter(|b| b.len() == 16) else {
        return Outcome::Gone;
    };
    let builder = WebPushBuilder::new(uri, key, Auth::clone_from_slice(&auth)).with_vapid(pair, CONTACT);
    let Ok(request) = builder.build(payload) else {
        return Outcome::Failed;
    };
    match agent.run(request) {
        Ok(resp) => match resp.status().as_u16() {
            200..=299 => Outcome::Sent,
            404 | 410 => Outcome::Gone,
            _ => Outcome::Failed,
        },
        Err(_) => Outcome::Failed,
    }
}