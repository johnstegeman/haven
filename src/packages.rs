#[derive(Debug, Clone)]
pub struct OutdatedPackage {
    pub name: String,
    pub current_version: String,
    pub latest_version: String,
}
