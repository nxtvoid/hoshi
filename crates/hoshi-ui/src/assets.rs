//! Icons: gpui-kit's component set plus the few hoshi adds from the same Lucide catalog.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

pub use gpui_kit::assets::IconName;

gpui_kit::assets::icon_assets!(
    Extra,
    [
        AlarmClock,
        AlarmClockOff,
        AppWindow,
        ArrowLeft,
        ArrowRightLeft,
        ArrowUpLeft,
        ArrowUpRight,
        Bot,
        Brain,
        Brush,
        ChartColumn,
        Check,
        ChevronDown,
        ChevronRight,
        ChevronsDownUp,
        Circle,
        CircleAlert,
        CircleCheck,
        CircleQuestionMark,
        CircleStop,
        Clock,
        CloudDownload,
        CloudUpload,
        Code,
        CornerUpLeft,
        Download,
        Copy,
        CornerDownRight,
        Ellipsis,
        ExternalLink,
        Eye,
        File,
        FileDiff,
        FilePen,
        FileText,
        Folder,
        FolderGit,
        FolderPlus,
        GitBranch,
        GitCommitHorizontal,
        GitFork,
        GitMerge,
        GitMergeConflict,
        GitPullRequest,
        GitPullRequestClosed,
        GitPullRequestDraft,
        Github,
        Globe,
        Github,
        GripVertical,
        Image,
        Inbox,
        Infinity,
        KeyRound,
        Link,
        ListChecks,
        ListTodo,
        List,
        LoaderCircle,
        Lock,
        LockOpen,
        Maximize2,
        MessageCircle,
        MessageSquare,
        MessagesSquare,
        Minimize2,
        Monitor,
        MessageCircleQuestionMark,
        PanelLeft,
        Paperclip,
        Pencil,
        PictureInPicture2,
        Plug,
        RefreshCw,
        RotateCcw,
        RotateCw,
        Search,
        Send,
        Settings,
        Share2,
        Slash,
        SlidersHorizontal,
        Sparkles,
        SquarePen,
        SquareTerminal,
        ShieldUser,
        Shrink,
        Star,
        Tag,
        Ticket,
        Trash,
        TriangleAlert,
        Upload,
        UserPlus,
        Users,
        WandSparkles,
        Workflow,
        Wrench,
        X,
        Zap
    ]
);

/// hoshi's own marks, drawn in the text color like the icons.
pub mod brand {
    pub const WORDMARK: &str = "brand/wordmark.svg";
    pub const MARK: &str = "brand/mark.svg";
    pub const CLAUDE: &str = "brand/claude.svg";
    /// Tabler's GitHub Copilot mark (MIT).
    pub const COPILOT: &str = "brand/copilot.svg";
    /// Tabler's OpenAI mark (MIT), for Codex.
    pub const CODEX: &str = "brand/codex.svg";
    pub const BITBUCKET: &str = "brand/bitbucket.svg";
    /// Lucide's star, filled: a favorite.
    pub const STAR_FILLED: &str = "brand/star-filled.svg";

    /// The path of an editor's mark by its name in `assets/brand/editors`, like `vscode`. Cursor's
    /// and Zed's are one color, drawn in the text color; the others keep their own colors.
    pub fn editor(name: &str) -> String {
        format!("brand/editors/{name}.svg")
    }
}

macro_rules! editor_mark {
    ($name:literal) => {
        (
            concat!("brand/editors/", $name, ".svg"),
            include_bytes!(concat!("../assets/brand/editors/", $name, ".svg")).as_slice(),
        )
    };
}

const EDITOR_MARKS: [(&str, &[u8]); 14] = [
    editor_mark!("vscode"),
    editor_mark!("cursor"),
    editor_mark!("zed"),
    editor_mark!("file-explorer"),
    editor_mark!("finder"),
    editor_mark!("idea"),
    editor_mark!("webstorm"),
    editor_mark!("pycharm"),
    editor_mark!("goland"),
    editor_mark!("rider"),
    editor_mark!("clion"),
    editor_mark!("phpstorm"),
    editor_mark!("rubymine"),
    editor_mark!("rustrover"),
];

const BRAND: [(&str, &[u8]); 7] = [
    (
        brand::WORDMARK,
        include_bytes!("../assets/brand/wordmark.svg"),
    ),
    (brand::MARK, include_bytes!("../assets/brand/mark.svg")),
    (brand::CLAUDE, include_bytes!("../assets/brand/claude.svg")),
    (
        brand::COPILOT,
        include_bytes!("../assets/brand/copilot.svg"),
    ),
    (brand::CODEX, include_bytes!("../assets/brand/codex.svg")),
    (
        brand::BITBUCKET,
        include_bytes!("../assets/brand/bitbucket.svg"),
    ),
    (
        brand::STAR_FILLED,
        include_bytes!("../assets/brand/star-filled.svg"),
    ),
];

/// File-type icons from `@pierre/trees` (Apache-2.0, `assets/file-icons/LICENSE.md`), the set
/// the web UI shows; drawn in one color each, like the icons.
macro_rules! file_icon {
    ($name:literal) => {
        (
            concat!("file-icons/", $name, ".svg"),
            include_bytes!(concat!("../assets/file-icons/", $name, ".svg")).as_slice(),
        )
    };
}

const FILE_ICONS: [(&str, &[u8]); 21] = [
    file_icon!("bash"),
    file_icon!("c"),
    file_icon!("cpp"),
    file_icon!("css"),
    file_icon!("database"),
    file_icon!("default"),
    file_icon!("docker"),
    file_icon!("go"),
    file_icon!("html"),
    file_icon!("javascript"),
    file_icon!("json"),
    file_icon!("markdown"),
    file_icon!("python"),
    file_icon!("react"),
    file_icon!("ruby"),
    file_icon!("rust"),
    file_icon!("sass"),
    file_icon!("swift"),
    file_icon!("text"),
    file_icon!("typescript"),
    file_icon!("yml"),
];

/// The path of a file-type icon by its name in `assets/file-icons`, like `rust`.
pub fn file_icon(name: &str) -> String {
    format!("file-icons/{name}.svg")
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = BRAND
            .iter()
            .chain(FILE_ICONS.iter())
            .chain(EDITOR_MARKS.iter())
            .find(|(name, _)| *name == path)
        {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = Extra.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(Extra.list(path)?);
        paths.extend(
            BRAND
                .iter()
                .chain(FILE_ICONS.iter())
                .chain(EDITOR_MARKS.iter())
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::{Assets, IconName};
    use gpui_kit::AssetSource as _;

    #[test]
    fn serves_the_added_icons_and_the_component_ones() {
        for icon in [IconName::Brain, IconName::FilePen, IconName::ChevronRight] {
            let path = icon.path();
            assert!(Assets.load(&path).unwrap().is_some(), "{path}");
        }
        assert!(Assets.load(super::brand::WORDMARK).unwrap().is_some());
        assert!(Assets.load(&super::file_icon("rust")).unwrap().is_some());
        assert!(
            Assets
                .load(&super::brand::editor("vscode"))
                .unwrap()
                .is_some()
        );
    }
}
