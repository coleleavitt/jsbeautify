use super::macros::lazy_regex;
use super::{SecretRule, Severity};

lazy_regex!(P_SECRET, r"(?i)secret");
lazy_regex!(P_PASSWD, r"(?i)passw");
lazy_regex!(P_AUTHORIZATION, r"(?i)authorization");
lazy_regex!(P_AWS_ACCESS_KEY_ID, r"(?i)aws[_-]?access[_-]?key[_-]?id");
lazy_regex!(P_AWS_SECRET_ACCESS_KEY, r"(?i)aws[_-]?secret[_-]?access[_-]?key");
lazy_regex!(P_ACCESS, r"(?i)access[_-]?(?:key|token|secret)");
lazy_regex!(P_AUTH, r"(?i)auth[_-]?(?:key|token|secret)");
lazy_regex!(P_SESSION, r"(?i)session[_-]?(?:key|token|secret)");
lazy_regex!(P_CONSUMER, r"(?i)consumer[_-]?(?:key|token|secret)");
lazy_regex!(P_PUBLIC, r"(?i)public[_-]?(?:key|token|secret)");
lazy_regex!(P_CLIENT, r"(?i)client[_-]?(?:id|token|key|secret)");
lazy_regex!(P_SLACK_BOT, r"(?i)slack[_-]?bot[_-]?token");
lazy_regex!(P_SLACK, r"(?i)slack[_-]?token");
lazy_regex!(P_ID_DSA, r"(?i)id[_-]?(?:dsa|rsa|ecdsa|ed25519)");
lazy_regex!(P_SSH, r"(?i)ssh[_-]?(?:key|priv)");
lazy_regex!(P_ENCRYPT, r"(?i)encrypt[_-]?(?:secret|key)");
lazy_regex!(P_DECRYPT, r"(?i)decrypt[_-]?(?:secret|key)");
lazy_regex!(P_GITHUB, r"(?i)github[_-]?(?:key|token|secret)");
lazy_regex!(P_IRC_PASS, r"(?i)irc[_-]?pass");
lazy_regex!(P_API, r"(?i)api[_-]?(?:key|token|secret)");
lazy_regex!(P_PRIVATE_KEY, r"(?i)private[_-]?key");
lazy_regex!(
    P_DB_PASSWORD,
    r"(?i)(?:db|database|mysql|postgres|redis)[_-]?pass(?:word)?"
);

pub(super) fn append_rules(rules: &mut Vec<SecretRule>) {
    macro_rules! prop {
        ($kind:expr, $regex:expr) => {
            rules.push(SecretRule {
                kind: $kind,
                pattern: $regex.as_str(),
                regex: &$regex,
                is_property_name: true,
                severity: Severity::Medium,
            });
        };
    }

    prop!("sensitive_property_name:secret", P_SECRET);
    prop!("sensitive_property_name:passwd", P_PASSWD);
    prop!("sensitive_property_name:authorization", P_AUTHORIZATION);
    prop!("sensitive_property_name:aws_access_key_id", P_AWS_ACCESS_KEY_ID);
    prop!("sensitive_property_name:aws_secret_access_key", P_AWS_SECRET_ACCESS_KEY);
    prop!("sensitive_property_name:access", P_ACCESS);
    prop!("sensitive_property_name:auth", P_AUTH);
    prop!("sensitive_property_name:session", P_SESSION);
    prop!("sensitive_property_name:consumer", P_CONSUMER);
    prop!("sensitive_property_name:public", P_PUBLIC);
    prop!("sensitive_property_name:client", P_CLIENT);
    prop!("sensitive_property_name:slack_bot", P_SLACK_BOT);
    prop!("sensitive_property_name:slack", P_SLACK);
    prop!("sensitive_property_name:id_dsa", P_ID_DSA);
    prop!("sensitive_property_name:ssh", P_SSH);
    prop!("sensitive_property_name:encrypt", P_ENCRYPT);
    prop!("sensitive_property_name:decrypt", P_DECRYPT);
    prop!("sensitive_property_name:github", P_GITHUB);
    prop!("sensitive_property_name:irc_pass", P_IRC_PASS);
    prop!("sensitive_property_name:api", P_API);
    prop!("sensitive_property_name:private_key", P_PRIVATE_KEY);
    prop!("sensitive_property_name:db_password", P_DB_PASSWORD);
}
