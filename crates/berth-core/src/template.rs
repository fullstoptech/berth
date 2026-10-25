use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateKind {
    BuiltIn,
    Custom,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateInputKind {
    Port { default: u16 },
    Password,
    VolumeName { default: String },
    Version { choices: Vec<String> },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateInput {
    pub key: String,
    pub label: String,
    pub kind: TemplateInputKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub kind: TemplateKind,
    pub service: String,
    pub env: Vec<(String, String)>,
    pub inputs: Vec<TemplateInput>,
    pub category: String,
    pub image: String,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_round_trip() {
        let t = Template {
            id: "postgres".to_string(),
            name: "Postgres".to_string(),
            kind: TemplateKind::BuiltIn,
            service: "image: postgres:${VERSION}".to_string(),
            env: vec![],
            inputs: vec![TemplateInput {
                key: "PORT".to_string(),
                label: "Port".to_string(),
                kind: TemplateInputKind::Port { default: 5432 },
            }],
            category: "Database".to_string(),
            image: "postgres:17".to_string(),
            description: "db".to_string(),
        };
        let json = serde_json::to_string(&t).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Template>(&json).expect("deserialize"),
            t
        );
    }
}
