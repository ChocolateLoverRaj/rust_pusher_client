use futures_util::FutureExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::{AuthProvider, AuthRequest, PresenceChannelAuthRequest, PrivateChannelAuthRequest};

pub struct PrivateKeyAuthProvider {
    pub app_key: String,
    pub app_secret: String,
}

impl AuthProvider for PrivateKeyAuthProvider {
    fn get_auth_signature(
        &self,
        auth_request: crate::AuthRequest,
    ) -> futures_util::future::BoxFuture<crate::AuthResult> {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.app_secret.as_bytes()).unwrap();
        let string_to_sign = match auth_request {
            AuthRequest::Private(PrivateChannelAuthRequest { socket_id, channel }) => {
                format!("{}:{}", socket_id, channel)
            }
            AuthRequest::Presence(PresenceChannelAuthRequest {
                socket_id,
                channel,
                user_data,
            }) => {
                let user_data = serde_json::to_string(&user_data.map_for_pusher()).unwrap();
                format!("{}:{}:{}", socket_id, channel, user_data)
            }
        };
        println!("Signing string: {:?}", string_to_sign);
        mac.update(string_to_sign.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        let auth = format!("{}:{}", self.app_key, signature);
        async { Ok(auth) }.boxed()
    }
}
