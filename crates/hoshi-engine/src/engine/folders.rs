use crate::*;

impl Engine {
    /// The folders inside `path`, to choose where a project lives.
    pub fn list_folders(&self, path: &str) -> Result<FolderListing, CommandError> {
        folders::list(path)
    }

    /// Creates a folder for a project; with `init_repository`, a new repository in an
    /// empty folder. Returns its normalized path.
    pub fn create_folder(&self, path: &str, init_repository: bool) -> Result<String, CommandError> {
        folders::create(path, init_repository)
    }

    /// Clones into a new or empty folder and returns its normalized path. Takes as long
    /// as the download.
    pub fn clone_repository(&self, url: &str, destination: &str) -> Result<String, CommandError> {
        folders::clone(url, destination)
    }

    /// Looks up a repository typed as `owner/name` or a link. Asks the forge.
    pub fn find_repository(
        &self,
        forge: ForgeKind,
        input: &str,
    ) -> Result<RemoteRepository, CommandError> {
        let repository = parse_repository_name(forge, input).ok_or_else(|| {
            CommandError::RepositoryNameInvalid {
                input: input.to_owned(),
            }
        })?;
        let adapter = lock(&self.state)
            .forges
            .get(&forge)
            .cloned()
            .ok_or(CommandError::ForgeUnavailable)?;
        adapter.repository(&repository).map_err(forge_error)
    }

    /// The project's icon as a data URL, found among its files. Reads the disk.
    pub fn project_icon(&self, project_id: &str) -> Option<String> {
        let root = lock(&self.state)
            .model
            .project(project_id)?
            .workspace_root
            .clone();
        project_icon::data_url(Path::new(&root))
    }

    /// Where the project's icon is among its files. Reads the disk.
    pub fn project_icon_path(&self, project_id: &str) -> Option<std::path::PathBuf> {
        let root = lock(&self.state)
            .model
            .project(project_id)?
            .workspace_root
            .clone();
        project_icon::find(Path::new(&root))
    }

    /// The folders the enabled agents ran sessions in, newest first, read from their own
    /// history. Reads many files, so it takes a moment.
    pub fn recent_folders(&self) -> Vec<RecentFolder> {
        let (adapters, data_dir, roots) = {
            let state = lock(&self.state);
            let data_dir = state
                .worktrees_dir
                .parent()
                .map_or_else(|| state.worktrees_dir.clone(), Path::to_path_buf);
            let roots: Vec<String> = state
                .model
                .projects()
                .map(|project| project.workspace_root.clone())
                .collect();
            (state.adapters.clone(), data_dir, roots)
        };
        let found = thread::scope(|scope| {
            let scans: Vec<_> = adapters
                .iter()
                .map(|adapter| {
                    scope.spawn(|| (adapter.provider().to_owned(), adapter.session_folders()))
                })
                .collect();
            scans
                .into_iter()
                .filter_map(|scan| scan.join().ok())
                .collect()
        });
        let home = std::env::home_dir();
        recent_folders::gather(
            found,
            &recent_folders::Excluded::new(home.as_deref(), &data_dir, &std::env::temp_dir()),
            &roots,
        )
    }
}
