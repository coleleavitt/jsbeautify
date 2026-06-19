#[derive(Debug, Clone)]
pub struct WebpackModule {
    pub id: usize,
    pub start_pos: usize,
    pub end_pos: usize,
    pub dependencies: Vec<usize>,
}
