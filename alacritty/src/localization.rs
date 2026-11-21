use std::env;

use serde::{Deserialize, Deserializer, Serialize};

use alacritty_config_derive::SerdeReplace;

#[derive(SerdeReplace, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Language {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "pt-BR")]
    PtBr,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::Auto, Self::En, Self::PtBr];

    pub fn resolve(self) -> Locale {
        match self {
            Self::En => Locale::En,
            Self::PtBr => Locale::PtBr,
            Self::Auto => resolve_from(
                env::var("LC_ALL").ok().as_deref(),
                env::var("LC_MESSAGES").ok().as_deref(),
                env::var("LANG").ok().as_deref(),
                env::var("LANGUAGE").ok().as_deref(),
                system_locale().as_deref(),
            ),
        }
    }

    pub fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::Auto => locale.text(Message::Automatic),
            Self::En => "English",
            Self::PtBr => "Português (Brasil)",
        }
    }
}

impl<'de> Deserialize<'de> for Language {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        let value = value.trim().replace('_', "-").to_lowercase();
        match value.as_str() {
            "auto" | "system" => Ok(Self::Auto),
            "en" | "en-us" | "en-gb" | "english" => Ok(Self::En),
            "pt" | "pt-br" | "pt-pt" | "português" | "portugues" => Ok(Self::PtBr),
            _ => Err(serde::de::Error::custom("language must be auto, en, or pt-BR")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    En,
    PtBr,
}

fn locale_from_tag(tag: &str) -> Option<Locale> {
    let tag = tag.trim().split(['.', '@']).next()?.replace('_', "-").to_lowercase();
    match tag.split('-').next()? {
        "en" | "c" | "posix" => Some(Locale::En),
        "pt" => Some(Locale::PtBr),
        _ => None,
    }
}

fn resolve_from(
    lc_all: Option<&str>,
    lc_messages: Option<&str>,
    lang: Option<&str>,
    language: Option<&str>,
    system: Option<&str>,
) -> Locale {
    let base = [lc_all, lc_messages, lang].into_iter().flatten().find(|tag| !tag.trim().is_empty());
    if let Some(base) = base {
        let tag = base.trim().split(['.', '@']).next().unwrap_or_default();
        if tag.eq_ignore_ascii_case("c") || tag.eq_ignore_ascii_case("posix") {
            return Locale::En;
        }
        if let Some(locale) = language.and_then(|tags| tags.split(':').find_map(locale_from_tag)) {
            return locale;
        }
        return locale_from_tag(base).unwrap_or(Locale::En);
    }
    language
        .and_then(|tags| tags.split(':').find_map(locale_from_tag))
        .or_else(|| system.and_then(locale_from_tag))
        .unwrap_or(Locale::En)
}

#[cfg(not(windows))]
fn system_locale() -> Option<String> {
    None
}

#[cfg(windows)]
fn system_locale() -> Option<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(locale_name: *mut u16, locale_name_count: i32) -> i32;
    }

    let mut name = [0u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    (length > 1).then(|| String::from_utf16_lossy(&name[..length as usize - 1]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    AgentChats,
    Applications,
    PreferredEditor,
    PreferredFileManager,
    AppAutomatic,
    SystemDefault,
    AppNotInstalled,
    AppSettingsHint,
    OpenIn,
    OpenEditor,
    OpenFolder,
    DesktopOpenFailed,
    MenuHeading,
    NewTab,
    Rename,
    SaveAll,
    Restore,
    Copy,
    Paste,
    Close,
    ConfirmTabClose,
    CloseTabTitle,
    CloseTabQuestion,
    CloseTabWarning,
    CloseTabAccept,
    CloseTabReject,
    CloseTabKeys,
    Settings,
    DialogTitle,
    DialogSubtitle,
    Name,
    NameHelp,
    WorkingDirectory,
    DirectoryPrefix,
    Unavailable,
    RecentCommands,
    NoCommands,
    Replay,
    ReplayUnavailable,
    ReplayNoCommand,
    ReplayHelp,
    Apply,
    SaveTabs,
    Cancel,
    RestoreShort,
    KeyboardHelp,
    KeyboardHelpCompact,
    SavedShellFailed,
    SessionReadFailed,
    StateDirectoryFailed,
    SessionSaved,
    SessionSaveFailed,
    NoSavedSession,
    SessionRestoreFailed,
    ShellNotReady,
    CommandDirectoryMissing,
    ReplayFailed,
    SettingsTitle,
    SettingsSubtitle,
    InterfaceLanguage,
    Automatic,
    LanguageHint,
    SaveLanguage,
    LanguageSaved,
    LanguageSaveFailed,
    SearchForward,
    SearchBackward,
    TruncatedNotice,
    Appearance,
    AppIcon,
    IconOriginal,
    IconCat,
    IconDog,
    IconFox,
    IconRobot,
    IconYorkie,
    IconRestore,
    IconHint,
    IconIntegrationFailed,
    Startup,
    Shortcuts,
    FontFamily,
    FontSize,
    Opacity,
    Theme,
    CurrentTheme,
    DarkTheme,
    LightTheme,
    Defaults,
    RestoreAtStartup,
    AutoRecovery,
    StartupWorkspace,
    LastSavedSession,
    RecoveryHint,
    DefaultShortcuts,
    DuplicateTab,
    ReopenClosedTab,
    Workspaces,
    MoveTab,
    SaveSettings,
    SettingsInvalid,
    WorkspaceName,
    NoWorkspaces,
    SaveWorkspace,
    OpenWorkspace,
    UseAtStartup,
    ConfirmDelete,
    DeleteWorkspace,
    WorkspaceInvalid,
    PreferencesSubtitle,
    WorkspacesHint,
    WorkspaceSaved,
    WorkspaceFailed,
    WorkspaceDeleted,
    StartupSaved,
    NoClosedTabs,
    RecoveryFailed,
    GeneralPreferences,
    SettingsSaved,
    SettingsSaveFailed,
    SavingSettings,
    SavedButton,
    RetryButton,
    RecordShortcut,
    SwapShortcuts,
    ShortcutSwapHint,
    Terminal,
    Tabs,
    Sessions,
    ScrollbackLines,
    HideMouseTyping,
    CopyOnSelect,
    Updates,
    AutoCheckUpdates,
    CheckUpdatesNow,
    InstallUpdate,
    UpdateChecking,
    UpdateCurrent,
    UpdateAvailable,
    UpdateInstalling,
    UpdateInstalled,
    UpdateFailed,
    UpdateUnmanaged,
    UpdateNotice,
    TabSettingsHint,
    SettingsLanguageHint,
    SplitVertical,
    SplitHorizontal,
    FocusPane,
    PreviousTab,
    NextTab,
    TogglePaneZoom,
    ClosePane,
    PaneLimit,
    ClosePaneTitle,
    ClosePaneQuestion,
    QuitTitle,
    QuitQuestion,
    QuitAccept,
    ScrollbackSaveHint,
    LanguagePage,
    ShortcutEditHint,
    ShortcutCapture,
    ShortcutConflict,
    ShortcutInvalid,
    ShortcutDisabled,
    ShortcutClear,
    ShortcutChanged,
    ShortcutCancelCapture,
    ShortcutCurrent,
    ShortcutsSubtitle,
    ShortcutSearch,
    ShortcutAction,
    ShortcutKey,
    ShortcutNoResults,
    TabGroups,
    GroupsHint,
    GroupName,
    NoGroups,
    CreateGroup,
    RenameGroup,
    MoveToGroup,
    ShowGroup,
    UngroupTab,
    AllTabs,
    GroupInvalid,
    GroupDuplicate,
    Agents,
    AgentOverviewHint,
    AgentEmpty,
    AgentStartHint,
    AgentIntegration,
    AgentNotifications,
    AgentStatusIndicator,
    AgentAnimatedIndicator,
    AgentStaticIndicator,
    AgentSettingsHint,
    AgentNewTabsHint,
    AgentCodexHint,
    AgentLimited,
    AgentStatus,
    AgentClose,
    AgentReady,
    AgentWorking,
    AgentInput,
    AgentPermission,
    AgentDone,
    AgentError,
    AgentUnknown,
}

impl Locale {
    pub fn text(self, message: Message) -> &'static str {
        match (self, message) {
            (Self::En, Message::AgentChats) => "Chats",
            (Self::PtBr, Message::AgentChats) => "Conversas",
            (Self::En, Message::Applications) => "Applications",
            (Self::PtBr, Message::Applications) => "Aplicativos",
            (Self::En, Message::PreferredEditor) => "IDE / editor",
            (Self::PtBr, Message::PreferredEditor) => "IDE / editor de código",
            (Self::En, Message::PreferredFileManager) => "File manager",
            (Self::PtBr, Message::PreferredFileManager) => "Gerenciador de arquivos",
            (Self::En, Message::AppAutomatic) => "Automatic (first installed)",
            (Self::PtBr, Message::AppAutomatic) => "Automático (primeiro instalado)",
            (Self::En, Message::SystemDefault) => "System default",
            (Self::PtBr, Message::SystemDefault) => "Padrão do sistema",
            (Self::En, Message::AppNotInstalled) => "not installed",
            (Self::PtBr, Message::AppNotInstalled) => "não instalado",
            (Self::En, Message::AppSettingsHint) => {
                "Choose an installed app to open the terminal folder from its right-click menu. \
                 Save to apply your choices."
            },
            (Self::PtBr, Message::AppSettingsHint) => {
                "Escolha um aplicativo instalado para abrir a pasta do terminal pelo botão \
                 direito. Salve para aplicar suas escolhas."
            },
            (Self::En, Message::OpenIn) => "Open in",
            (Self::PtBr, Message::OpenIn) => "Abrir no",
            (Self::En, Message::OpenEditor) => "Open in editor…",
            (Self::PtBr, Message::OpenEditor) => "Abrir no editor…",
            (Self::En, Message::OpenFolder) => "Open folder…",
            (Self::PtBr, Message::OpenFolder) => "Abrir pasta…",
            (Self::En, Message::DesktopOpenFailed) => {
                "Could not open this folder. Check the app in Settings > Applications."
            },
            (Self::PtBr, Message::DesktopOpenFailed) => {
                "Não foi possível abrir esta pasta. Verifique o aplicativo em Configurações > \
                 Aplicativos."
            },
            (Self::En, Message::Agents) => "Agents",
            (Self::PtBr, Message::Agents) => "Agentes",
            (Self::En, Message::AgentOverviewHint) => "Select an agent to open its tab and pane.",
            (Self::PtBr, Message::AgentOverviewHint) => {
                "Selecione um agente para abrir sua aba e painel."
            },
            (Self::En, Message::AgentEmpty) => "No agents are connected in this window yet.",
            (Self::PtBr, Message::AgentEmpty) => "Nenhum agente conectado nesta janela ainda.",
            (Self::En, Message::AgentStartHint) => {
                "Start claude, codex, agy or opencode in a new tab."
            },
            (Self::PtBr, Message::AgentStartHint) => {
                "Inicie claude, codex, agy ou opencode em uma nova aba."
            },
            (Self::En, Message::AgentIntegration) => "Show agent status in tabs",
            (Self::PtBr, Message::AgentIntegration) => "Mostrar estado dos agentes nas abas",
            (Self::En, Message::AgentNotifications) => "Notify on waits and completion",
            (Self::PtBr, Message::AgentNotifications) => "Avisar sobre espera e conclusão",
            (Self::En, Message::AgentStatusIndicator) => "Status indicator",
            (Self::PtBr, Message::AgentStatusIndicator) => "Indicador de estado",
            (Self::En, Message::AgentAnimatedIndicator) => "Animation",
            (Self::PtBr, Message::AgentAnimatedIndicator) => "Animação",
            (Self::En, Message::AgentStaticIndicator) => "Static icon",
            (Self::PtBr, Message::AgentStaticIndicator) => "Ícone fixo",
            (Self::En, Message::AgentSettingsHint) => {
                "See when an agent is working, needs a reply or permission, or has finished. \
                 Permission decisions stay with you."
            },
            (Self::PtBr, Message::AgentSettingsHint) => {
                "Veja quando um agente está trabalhando, precisa de resposta ou permissão, ou \
                 terminou. Você continua decidindo as permissões."
            },
            (Self::En, Message::AgentNewTabsHint) => {
                "Save changes, then open new tabs or panes to start the integration."
            },
            (Self::PtBr, Message::AgentNewTabsHint) => {
                "Salve as mudanças e abra novas abas ou painéis para iniciar a integração."
            },
            (Self::En, Message::AgentCodexHint) => {
                "Compatible Codex versions connect automatically; no extra setup is needed."
            },
            (Self::PtBr, Message::AgentCodexHint) => {
                "Versões compatíveis do Codex se integram automaticamente, sem configuração \
                 adicional."
            },
            (Self::En, Message::AgentLimited) => "Limited status",
            (Self::PtBr, Message::AgentLimited) => "Estado limitado",
            (Self::En, Message::AgentStatus) => "Status",
            (Self::PtBr, Message::AgentStatus) => "Estado",
            (Self::En, Message::AgentClose) => "Close",
            (Self::PtBr, Message::AgentClose) => "Fechar",
            (Self::En, Message::AgentReady) => "Ready",
            (Self::PtBr, Message::AgentReady) => "Pronto",
            (Self::En, Message::AgentWorking) => "Working",
            (Self::PtBr, Message::AgentWorking) => "Trabalhando",
            (Self::En, Message::AgentInput) => "Needs a reply",
            (Self::PtBr, Message::AgentInput) => "Aguardando resposta",
            (Self::En, Message::AgentPermission) => "Needs permission",
            (Self::PtBr, Message::AgentPermission) => "Aguardando permissão",
            (Self::En, Message::AgentDone) => "Finished",
            (Self::PtBr, Message::AgentDone) => "Concluído",
            (Self::En, Message::AgentError) => "Failed",
            (Self::PtBr, Message::AgentError) => "Falhou",
            (Self::En, Message::AgentUnknown) => "Status unavailable",
            (Self::PtBr, Message::AgentUnknown) => "Estado indisponível",

            (Self::En, Message::TabGroups) => "Tab groups",
            (Self::PtBr, Message::TabGroups) => "Grupos de abas",
            (Self::En, Message::GroupsHint) => {
                "Organize tabs by project. Running terminals keep working."
            },
            (Self::PtBr, Message::GroupsHint) => {
                "Organize abas por projeto. Os terminais continuam funcionando."
            },
            (Self::En, Message::GroupName) => "Group name",
            (Self::PtBr, Message::GroupName) => "Nome do grupo",
            (Self::En, Message::NoGroups) => "No groups yet. Create one for this tab.",
            (Self::PtBr, Message::NoGroups) => "Nenhum grupo. Crie um para esta aba.",
            (Self::En, Message::CreateGroup) => "Create group",
            (Self::PtBr, Message::CreateGroup) => "Criar grupo",
            (Self::En, Message::RenameGroup) => "Rename group",
            (Self::PtBr, Message::RenameGroup) => "Renomear grupo",
            (Self::En, Message::MoveToGroup) => "Move tab here",
            (Self::PtBr, Message::MoveToGroup) => "Mover aba para cá",
            (Self::En, Message::ShowGroup) => "Show group",
            (Self::PtBr, Message::ShowGroup) => "Mostrar grupo",
            (Self::En, Message::UngroupTab) => "Remove from group",
            (Self::PtBr, Message::UngroupTab) => "Tirar do grupo",
            (Self::En, Message::AllTabs) => "All tabs",
            (Self::PtBr, Message::AllTabs) => "Todas as abas",
            (Self::En, Message::GroupInvalid) => "Use a name with 1 to 64 characters.",
            (Self::PtBr, Message::GroupInvalid) => "Use um nome de 1 a 64 caracteres.",
            (Self::En, Message::GroupDuplicate) => "A group already has this name.",
            (Self::PtBr, Message::GroupDuplicate) => "Já existe um grupo com esse nome.",

            (Self::En, Message::ShortcutSearch) => "Search actions or keys…",
            (Self::PtBr, Message::ShortcutSearch) => "Buscar ação ou tecla…",
            (Self::En, Message::ShortcutAction) => "Action",
            (Self::PtBr, Message::ShortcutAction) => "Ação",
            (Self::En, Message::ShortcutKey) => "Shortcut",
            (Self::PtBr, Message::ShortcutKey) => "Atalho",
            (Self::En, Message::ShortcutNoResults) => "No matching shortcuts.",
            (Self::PtBr, Message::ShortcutNoResults) => "Nenhum atalho encontrado.",
            (Self::En, Message::ShortcutEditHint) => "Select an action to change its shortcut.",
            (Self::PtBr, Message::ShortcutEditHint) => "Selecione a ação para alterar o atalho.",
            (Self::En, Message::ShortcutCapture) => "Press a shortcut; Esc cancels.",
            (Self::PtBr, Message::ShortcutCapture) => "Pressione o atalho; Esc cancela.",
            (Self::En, Message::ShortcutConflict) => "Shortcut already used. Choose another.",
            (Self::PtBr, Message::ShortcutConflict) => "Atalho em uso. Escolha outro.",
            (Self::En, Message::ShortcutInvalid) => "Use Ctrl, Alt, Super or a function key.",
            (Self::PtBr, Message::ShortcutInvalid) => "Use Ctrl, Alt, Super ou uma tecla F.",
            (Self::En, Message::ShortcutDisabled) => "Disabled",
            (Self::PtBr, Message::ShortcutDisabled) => "Desativado",
            (Self::En, Message::ShortcutClear) => "Disable shortcut",
            (Self::PtBr, Message::ShortcutClear) => "Desativar atalho",
            (Self::En, Message::ShortcutChanged) => "Shortcut changed. Save to apply.",
            (Self::PtBr, Message::ShortcutChanged) => "Atalho alterado. Salve para aplicar.",
            (Self::En, Message::ShortcutCancelCapture) => "Cancel capture",
            (Self::PtBr, Message::ShortcutCancelCapture) => "Cancelar captura",
            (Self::En, Message::ShortcutCurrent) => "Current shortcut",
            (Self::PtBr, Message::ShortcutCurrent) => "Atalho atual",
            (Self::En, Message::ShortcutsSubtitle) => {
                "Click an action, press the new shortcut, then save."
            },
            (Self::PtBr, Message::ShortcutsSubtitle) => {
                "Clique na ação, pressione o novo atalho e salve."
            },

            (Self::En, Message::ScrollbackSaveHint) => "Applied when you save.",
            (Self::PtBr, Message::ScrollbackSaveHint) => "Aplicado ao salvar.",
            (Self::En, Message::LanguagePage) => "Language",
            (Self::PtBr, Message::LanguagePage) => "Idioma",

            (Self::En, Message::Terminal) => "Terminal",
            (Self::PtBr, Message::Terminal) => "Terminal",
            (Self::En, Message::Tabs) => "Tabs",
            (Self::PtBr, Message::Tabs) => "Abas",
            (Self::En, Message::Sessions) => "Sessions",
            (Self::PtBr, Message::Sessions) => "Sessões",
            (Self::En, Message::ScrollbackLines) => "Scrollback lines",
            (Self::PtBr, Message::ScrollbackLines) => "Linhas de histórico",
            (Self::En, Message::HideMouseTyping) => "Hide mouse while typing",
            (Self::PtBr, Message::HideMouseTyping) => "Ocultar mouse ao digitar",
            (Self::En, Message::CopyOnSelect) => "Copy selection to clipboard",
            (Self::PtBr, Message::CopyOnSelect) => "Copiar seleção automaticamente",
            (Self::En, Message::Updates) => "Updates",
            (Self::PtBr, Message::Updates) => "Atualizações",
            (Self::En, Message::AutoCheckUpdates) => "Check automatically",
            (Self::PtBr, Message::AutoCheckUpdates) => "Verificar automaticamente",
            (Self::En, Message::CheckUpdatesNow) => "Check now",
            (Self::PtBr, Message::CheckUpdatesNow) => "Verificar agora",
            (Self::En, Message::InstallUpdate) => "Install update",
            (Self::PtBr, Message::InstallUpdate) => "Instalar atualização",
            (Self::En, Message::UpdateChecking) => "Checking for updates…",
            (Self::PtBr, Message::UpdateChecking) => "Verificando atualizações…",
            (Self::En, Message::UpdateCurrent) => "Termust is up to date.",
            (Self::PtBr, Message::UpdateCurrent) => "O Termust está atualizado.",
            (Self::En, Message::UpdateAvailable) => "Termust {version} is available.",
            (Self::PtBr, Message::UpdateAvailable) => "O Termust {version} está disponível.",
            (Self::En, Message::UpdateInstalling) => "Installing update…",
            (Self::PtBr, Message::UpdateInstalling) => "Instalando atualização…",
            (Self::En, Message::UpdateInstalled) => {
                "Update installed. Reopen Termust to use the new version."
            },
            (Self::PtBr, Message::UpdateInstalled) => {
                "Atualização instalada. Reabra o Termust para usar a nova versão."
            },
            (Self::En, Message::UpdateFailed) => "The update failed. Check again to retry.",
            (Self::PtBr, Message::UpdateFailed) => "Falha na atualização. Verifique novamente.",
            (Self::En, Message::UpdateUnmanaged) => {
                "To update this copy of Termust, use the same method you used to install it."
            },
            (Self::PtBr, Message::UpdateUnmanaged) => {
                "Para atualizar esta cópia do Termust, use o mesmo método da instalação."
            },
            (Self::En, Message::UpdateNotice) => {
                "A Termust update is available. Open Settings → Terminal to install it."
            },
            (Self::PtBr, Message::UpdateNotice) => {
                "Há uma atualização do Termust. Abra Configurações → Terminal para instalá-la."
            },
            (Self::En, Message::TabSettingsHint) => {
                "Manage names and command replay in tab settings."
            },
            (Self::PtBr, Message::TabSettingsHint) => "Nome e reexecução nas configurações da aba.",
            (Self::En, Message::SettingsLanguageHint) => {
                "Changes interface language; shell output is unchanged."
            },
            (Self::PtBr, Message::SettingsLanguageHint) => {
                "Altera o idioma da interface; saída do shell mantida."
            },
            (Self::En, Message::SplitVertical) => "Split right",
            (Self::PtBr, Message::SplitVertical) => "Dividir à direita",
            (Self::En, Message::SplitHorizontal) => "Split below",
            (Self::PtBr, Message::SplitHorizontal) => "Dividir abaixo",
            (Self::En, Message::PreviousTab) => "Previous tab",
            (Self::PtBr, Message::PreviousTab) => "Aba anterior",
            (Self::En, Message::NextTab) => "Next tab",
            (Self::PtBr, Message::NextTab) => "Próxima aba",
            (Self::En, Message::FocusPane) => "Focus pane",
            (Self::PtBr, Message::FocusPane) => "Alternar painel",
            (Self::En, Message::TogglePaneZoom) => "Maximize / restore pane",
            (Self::PtBr, Message::TogglePaneZoom) => "Maximizar / restaurar painel",
            (Self::En, Message::ClosePane) => "Close pane",
            (Self::PtBr, Message::ClosePane) => "Fechar painel",
            (Self::En, Message::PaneLimit) => "This tab already has sixteen panes.",
            (Self::PtBr, Message::PaneLimit) => "Esta aba já tem dezesseis painéis.",
            (Self::En, Message::ClosePaneTitle) => "Close terminal pane",
            (Self::PtBr, Message::ClosePaneTitle) => "Fechar painel",
            (Self::En, Message::ClosePaneQuestion) => "Close this terminal pane?",
            (Self::PtBr, Message::ClosePaneQuestion) => "Fechar este painel?",
            (Self::En, Message::QuitTitle) => "Quit Termust",
            (Self::PtBr, Message::QuitTitle) => "Sair do Termust",
            (Self::En, Message::QuitQuestion) => "Close every window and tab?",
            (Self::PtBr, Message::QuitQuestion) => "Fechar todas as janelas e abas?",
            (Self::En, Message::QuitAccept) => "Quit",
            (Self::PtBr, Message::QuitAccept) => "Sair",

            (Self::En, Message::Appearance) => "Appearance",
            (Self::PtBr, Message::Appearance) => "Aparência",
            (Self::En, Message::AppIcon) => "Application icon",
            (Self::PtBr, Message::AppIcon) => "Ícone do aplicativo",
            (Self::En, Message::IconOriginal) => "Termust · Default",
            (Self::PtBr, Message::IconOriginal) => "Termust · Original",
            (Self::En, Message::IconCat) => "Cat",
            (Self::PtBr, Message::IconCat) => "Gato",
            (Self::En, Message::IconDog) => "Dog",
            (Self::PtBr, Message::IconDog) => "Cachorro",
            (Self::En, Message::IconFox) => "Fox",
            (Self::PtBr, Message::IconFox) => "Raposa",
            (Self::En, Message::IconRobot) => "Robot",
            (Self::PtBr, Message::IconRobot) => "Robô",
            (Self::En | Self::PtBr, Message::IconYorkie) => "Yorkie",
            (Self::En, Message::IconRestore) => "Restore original icon",
            (Self::PtBr, Message::IconRestore) => "Restaurar ícone original",
            #[cfg(target_os = "macos")]
            (Self::En, Message::IconHint) => {
                "Preview your Dock icon. Your choice applies while Termust is open; the Finder icon stays original."
            },
            #[cfg(target_os = "macos")]
            (Self::PtBr, Message::IconHint) => {
                "Veja a prévia no Dock. A escolha vale enquanto o Termust está aberto; o Finder mantém o ícone original."
            },
            #[cfg(windows)]
            (Self::En, Message::IconHint) => {
                "Preview your window icon. Save to update the shortcut; pinned icons may need to be pinned again."
            },
            #[cfg(windows)]
            (Self::PtBr, Message::IconHint) => {
                "Veja a prévia na janela. Salve para atualizar o atalho; ícones fixados podem precisar ser fixados novamente."
            },
            #[cfg(not(any(windows, target_os = "macos")))]
            (Self::En, Message::IconHint) => {
                "Make Termust your own. Save to update the launcher; pinned icons may need to be pinned again."
            },
            #[cfg(not(any(windows, target_os = "macos")))]
            (Self::PtBr, Message::IconHint) => {
                "Deixe o Termust com a sua cara. Salve para mudar o lançador; ícones fixados podem precisar ser fixados novamente."
            },
            (Self::En, Message::IconIntegrationFailed) => {
                "Settings saved. The launcher icon could not be updated; check its permissions."
            },
            (Self::PtBr, Message::IconIntegrationFailed) => {
                "Configurações salvas. Não foi possível mudar o ícone do lançador; verifique suas permissões."
            },
            (Self::En, Message::Startup) => "Startup",
            (Self::PtBr, Message::Startup) => "Inicialização",
            (Self::En, Message::Shortcuts) => "Shortcuts",
            (Self::PtBr, Message::Shortcuts) => "Atalhos",
            (Self::En, Message::FontFamily) => "Font family",
            (Self::PtBr, Message::FontFamily) => "Família da fonte",
            (Self::En, Message::FontSize) => "Font size",
            (Self::PtBr, Message::FontSize) => "Tamanho da fonte",
            (Self::En, Message::Opacity) => "Opacity",
            (Self::PtBr, Message::Opacity) => "Opacidade",
            (Self::En, Message::Theme) => "Theme",
            (Self::PtBr, Message::Theme) => "Tema",
            (Self::En, Message::CurrentTheme) => "Keep current colors",
            (Self::PtBr, Message::CurrentTheme) => "Manter cores atuais",
            (Self::En, Message::DarkTheme) => "Dark",
            (Self::PtBr, Message::DarkTheme) => "Escuro",
            (Self::En, Message::LightTheme) => "Light",
            (Self::PtBr, Message::LightTheme) => "Claro",
            (Self::En, Message::Defaults) => "Restore defaults",
            (Self::PtBr, Message::Defaults) => "Restaurar padrões",
            (Self::En, Message::RestoreAtStartup) => "Restore tabs at startup",
            (Self::PtBr, Message::RestoreAtStartup) => "Restaurar abas ao iniciar",
            (Self::En, Message::AutoRecovery) => "Save terminal state automatically",
            (Self::PtBr, Message::AutoRecovery) => "Salvar estado do terminal automaticamente",
            (Self::En, Message::StartupWorkspace) => "Startup workspace",
            (Self::PtBr, Message::StartupWorkspace) => "Ambiente inicial",
            (Self::En, Message::LastSavedSession) => "Last saved session",
            (Self::PtBr, Message::LastSavedSession) => "Última sessão salva",
            (Self::En, Message::RecoveryHint) => {
                "Saves tabs, groups, folders, last commands and recent output. Recovery starts \
                 fresh shells without running commands again."
            },
            (Self::PtBr, Message::RecoveryHint) => {
                "Salva abas, grupos, pastas, últimos comandos e saída recente. A recuperação abre \
                 novos shells sem executar comandos novamente."
            },
            (Self::En, Message::DefaultShortcuts) => {
                "Default shortcuts; custom bindings use the config file."
            },
            (Self::PtBr, Message::DefaultShortcuts) => {
                "Atalhos padrão; personalize pelo arquivo de configuração."
            },
            (Self::En, Message::DuplicateTab) => "Duplicate tab",
            (Self::PtBr, Message::DuplicateTab) => "Duplicar aba",
            (Self::En, Message::ReopenClosedTab) => "Reopen closed tab",
            (Self::PtBr, Message::ReopenClosedTab) => "Reabrir aba fechada",
            (Self::En, Message::Workspaces) => "Workspaces…",
            (Self::PtBr, Message::Workspaces) => "Ambientes de trabalho…",
            (Self::En, Message::MoveTab) => "Move tab",
            (Self::PtBr, Message::MoveTab) => "Mover aba",
            (Self::En, Message::SaveSettings) => "Save",
            (Self::PtBr, Message::SaveSettings) => "Salvar",
            (Self::En, Message::SettingsInvalid) => "Check the font and numeric settings.",
            (Self::PtBr, Message::SettingsInvalid) => {
                "Confira a fonte e os valores das configurações."
            },
            (Self::En, Message::WorkspaceName) => "Workspace name",
            (Self::PtBr, Message::WorkspaceName) => "Nome do ambiente",
            (Self::En, Message::NoWorkspaces) => "No named workspaces yet.",
            (Self::PtBr, Message::NoWorkspaces) => "Nenhum ambiente salvo ainda.",
            (Self::En, Message::SaveWorkspace) => "Save current tabs",
            (Self::PtBr, Message::SaveWorkspace) => "Salvar abas atuais",
            (Self::En, Message::OpenWorkspace) => "Open saved tabs",
            (Self::PtBr, Message::OpenWorkspace) => "Abrir abas salvas",
            (Self::En, Message::UseAtStartup) => "Use at startup",
            (Self::PtBr, Message::UseAtStartup) => "Usar ao iniciar",
            (Self::En, Message::ConfirmDelete) => "Confirm removal",
            (Self::PtBr, Message::ConfirmDelete) => "Confirmar remoção",
            (Self::En, Message::DeleteWorkspace) => "Remove workspace",
            (Self::PtBr, Message::DeleteWorkspace) => "Remover ambiente",
            (Self::En, Message::WorkspaceInvalid) => "Enter a name with 1–80 characters.",
            (Self::PtBr, Message::WorkspaceInvalid) => "Informe um nome com 1 a 80 caracteres.",
            (Self::En, Message::PreferencesSubtitle) => "Preview changes, then save or cancel.",
            (Self::PtBr, Message::PreferencesSubtitle) => "Ajuste, visualize e salve.",
            (Self::En, Message::WorkspacesHint) => {
                "Save layouts; opening adds tabs to this window."
            },
            (Self::PtBr, Message::WorkspacesHint) => {
                "Salve ambientes; abrir adiciona abas nesta janela."
            },
            (Self::En, Message::WorkspaceSaved) => "Workspace saved.",
            (Self::PtBr, Message::WorkspaceSaved) => "Ambiente salvo.",
            (Self::En, Message::WorkspaceFailed) => {
                "Could not access workspaces; saved data was preserved."
            },
            (Self::PtBr, Message::WorkspaceFailed) => {
                "Não foi possível acessar os ambientes; os dados foram preservados."
            },
            (Self::En, Message::WorkspaceDeleted) => "Workspace removed.",
            (Self::PtBr, Message::WorkspaceDeleted) => "Ambiente removido.",
            (Self::En, Message::StartupSaved) => "Startup workspace saved.",
            (Self::PtBr, Message::StartupSaved) => "Ambiente inicial salvo.",
            (Self::En, Message::NoClosedTabs) => "No recently closed tabs.",
            (Self::PtBr, Message::NoClosedTabs) => "Nenhuma aba fechada recentemente.",
            (Self::En, Message::RecoveryFailed) => {
                "Could not save recovery; previous snapshot was preserved."
            },
            (Self::PtBr, Message::RecoveryFailed) => {
                "Não foi possível salvar a recuperação; a anterior foi preservada."
            },
            (Self::En, Message::GeneralPreferences) => "Appearance and startup…  F2",
            (Self::PtBr, Message::GeneralPreferences) => "Aparência e inicialização…  F2",
            (Self::En, Message::SavingSettings) => "Saving…",
            (Self::PtBr, Message::SavingSettings) => "Salvando",
            (Self::En, Message::SavedButton) => "✓ Saved",
            (Self::PtBr, Message::SavedButton) => "✓ Salvo",
            (Self::En, Message::RetryButton) => "Retry",
            (Self::PtBr, Message::RetryButton) => "Repetir",
            (Self::En, Message::SwapShortcuts) => "Swap shortcuts",
            (Self::PtBr, Message::SwapShortcuts) => "Trocar atalhos",
            (Self::En, Message::ShortcutSwapHint) => "Enter swaps shortcuts. Esc cancels.",
            (Self::PtBr, Message::ShortcutSwapHint) => "Enter troca os atalhos. Esc cancela.",
            (Self::En, Message::RecordShortcut) => "Press keys…",
            (Self::PtBr, Message::RecordShortcut) => "Pressione as teclas…",
            (Self::En, Message::SettingsSaved) => "Settings saved.",
            (Self::PtBr, Message::SettingsSaved) => "Configurações salvas.",
            (Self::En, Message::SettingsSaveFailed) => {
                "Could not save settings; configuration was preserved."
            },
            (Self::PtBr, Message::SettingsSaveFailed) => {
                "Não foi possível salvar; a configuração foi preservada."
            },

            (Self::En, Message::SearchForward) => "Search: ",
            (Self::PtBr, Message::SearchForward) => "Busca: ",
            (Self::En, Message::SearchBackward) => "Backward Search: ",
            (Self::PtBr, Message::SearchBackward) => "Busca inversa: ",
            (Self::En, Message::TruncatedNotice) => "[MESSAGE TRUNCATED]",
            (Self::PtBr, Message::TruncatedNotice) => "[MENSAGEM TRUNCADA]",
            (Self::En, Message::MenuHeading) => "TERMINAL ACTIONS",
            (Self::PtBr, Message::MenuHeading) => "AÇÕES DO TERMINAL",
            (Self::En, Message::NewTab) => "New tab",
            (Self::PtBr, Message::NewTab) => "Nova aba",
            (Self::En, Message::Rename) => "Rename and configure…",
            (Self::PtBr, Message::Rename) => "Renomear e configurar…",
            (Self::En, Message::SaveAll) => "Save all tabs",
            (Self::PtBr, Message::SaveAll) => "Salvar todas as abas",
            (Self::En, Message::Restore) => "Reopen saved session",
            (Self::PtBr, Message::Restore) => "Reabrir sessão salva",
            (Self::En, Message::Copy) => "Copy selection",
            (Self::PtBr, Message::Copy) => "Copiar seleção",
            (Self::En, Message::Paste) => "Paste",
            (Self::PtBr, Message::Paste) => "Colar",
            (Self::En, Message::Close) => "Close tab",
            (Self::PtBr, Message::Close) => "Fechar aba",
            (Self::En, Message::ConfirmTabClose) => "Confirm before closing a tab",
            (Self::PtBr, Message::ConfirmTabClose) => "Confirmar antes de fechar uma aba",
            (Self::En, Message::CloseTabTitle) => "Close tab",
            (Self::PtBr, Message::CloseTabTitle) => "Fechar aba",
            (Self::En, Message::CloseTabQuestion) => "Close this tab?",
            (Self::PtBr, Message::CloseTabQuestion) => "Fechar esta aba?",
            (Self::En, Message::CloseTabWarning) => {
                "The selected terminal sessions will be closed."
            },
            (Self::PtBr, Message::CloseTabWarning) => "As sessões selecionadas serão encerradas.",
            (Self::En, Message::CloseTabAccept) => "Close tab",
            (Self::PtBr, Message::CloseTabAccept) => "Fechar aba",
            (Self::En, Message::CloseTabReject) => "Cancel",
            (Self::PtBr, Message::CloseTabReject) => "Cancelar",
            (Self::En, Message::CloseTabKeys) => "Y / Enter: close   N / Esc: cancel",
            (Self::PtBr, Message::CloseTabKeys) => "Y / Enter: fechar   N / Esc: cancelar",
            (Self::En, Message::Settings) => "Settings…",
            (Self::PtBr, Message::Settings) => "Configurações…",
            (Self::En, Message::DialogTitle) => "Edit tab",
            (Self::PtBr, Message::DialogTitle) => "Editar aba",
            (Self::En, Message::DialogSubtitle) => {
                "Name this tab and choose how to resume your work."
            },
            (Self::PtBr, Message::DialogSubtitle) => {
                "Dê um nome e escolha como retomar o trabalho."
            },
            (Self::En, Message::Name) => "Tab name",
            (Self::PtBr, Message::Name) => "Nome da aba",
            (Self::En, Message::NameHelp) => "Leave empty to use the automatic name.",
            (Self::PtBr, Message::NameHelp) => "Deixe vazio para usar o nome automático.",
            (Self::En, Message::WorkingDirectory) => "Working folder",
            (Self::PtBr, Message::WorkingDirectory) => "Pasta de trabalho",
            (Self::En, Message::DirectoryPrefix) => "Folder:",
            (Self::PtBr, Message::DirectoryPrefix) => "Pasta:",
            (Self::En, Message::Unavailable) => "Unavailable",
            (Self::PtBr, Message::Unavailable) => "Indisponível",
            (Self::En, Message::RecentCommands) => "Recent commands",
            (Self::PtBr, Message::RecentCommands) => "Comandos recentes",
            (Self::En, Message::NoCommands) => "No commands executed yet.",
            (Self::PtBr, Message::NoCommands) => "Nenhum comando executado ainda.",
            (Self::En, Message::Replay) => "Run the last command when reopening",
            (Self::PtBr, Message::Replay) => "Executar o último comando ao reabrir",
            (Self::En, Message::ReplayUnavailable) => {
                "Command history is unavailable for this shell."
            },
            (Self::PtBr, Message::ReplayUnavailable) => {
                "O histórico não está disponível neste shell."
            },
            (Self::En, Message::ReplayNoCommand) => "Run a command before enabling this option.",
            (Self::PtBr, Message::ReplayNoCommand) => {
                "Execute um comando antes de ativar esta opção."
            },
            (Self::En, Message::ReplayHelp) => "Reopening runs the command in its original folder.",
            (Self::PtBr, Message::ReplayHelp) => {
                "Ao reabrir, executar na pasta original do comando."
            },
            (Self::En, Message::Apply) => "Apply",
            (Self::PtBr, Message::Apply) => "Aplicar",
            (Self::En, Message::SaveTabs) => "Save tabs",
            (Self::PtBr, Message::SaveTabs) => "Salvar abas",
            (Self::En, Message::Cancel) => "Cancel",
            (Self::PtBr, Message::Cancel) => "Cancelar",
            (Self::En, Message::RestoreShort) => "Reopen session",
            (Self::PtBr, Message::RestoreShort) => "Reabrir sessão",
            (Self::En, Message::KeyboardHelp) => "Tab: next field   Enter: confirm   Esc: close",
            (Self::PtBr, Message::KeyboardHelp) => {
                "Tab: próximo campo   Enter: confirmar   Esc: fechar"
            },
            (Self::En, Message::KeyboardHelpCompact) => "Tab: next  Enter: OK  Esc: close",
            (Self::PtBr, Message::KeyboardHelpCompact) => "Tab: próximo  Enter: OK  Esc: fechar",
            (Self::En, Message::SavedShellFailed) => "Could not start the saved session's shell.",
            (Self::PtBr, Message::SavedShellFailed) => {
                "Não foi possível iniciar o shell da sessão salva."
            },
            (Self::En, Message::SessionReadFailed) => "Could not read the saved session.",
            (Self::PtBr, Message::SessionReadFailed) => "Não foi possível ler a sessão salva.",
            (Self::En, Message::StateDirectoryFailed) => "Could not find the user state directory.",
            (Self::PtBr, Message::StateDirectoryFailed) => {
                "Não foi possível localizar a pasta de dados do usuário."
            },
            (Self::En, Message::SessionSaved) => {
                "Session saved. Your tabs will reopen on the next startup."
            },
            (Self::PtBr, Message::SessionSaved) => {
                "Sessão salva. Suas abas serão reabertas na próxima inicialização."
            },
            (Self::En, Message::SessionSaveFailed) => {
                "Could not save the session. The previous file was preserved."
            },
            (Self::PtBr, Message::SessionSaveFailed) => {
                "Não foi possível salvar a sessão. O arquivo anterior foi preservado."
            },
            (Self::En, Message::NoSavedSession) => "No session has been saved yet.",
            (Self::PtBr, Message::NoSavedSession) => "Ainda não há uma sessão salva.",
            (Self::En, Message::SessionRestoreFailed) => "Could not reopen the saved tabs.",
            (Self::PtBr, Message::SessionRestoreFailed) => {
                "Não foi possível reabrir as abas salvas."
            },
            (Self::En, Message::ShellNotReady) => {
                "The shell was not ready; the saved command was not run."
            },
            (Self::PtBr, Message::ShellNotReady) => {
                "O shell não ficou pronto; o comando salvo não foi executado."
            },
            (Self::En, Message::CommandDirectoryMissing) => {
                "The command's folder no longer exists; rerun skipped."
            },
            (Self::PtBr, Message::CommandDirectoryMissing) => {
                "O diretório do comando não existe; reexecução ignorada."
            },
            (Self::En, Message::ReplayFailed) => "Could not prepare the command to run again.",
            (Self::PtBr, Message::ReplayFailed) => {
                "Não foi possível preparar a reexecução do comando."
            },
            (Self::En, Message::SettingsTitle) => "Settings",
            (Self::PtBr, Message::SettingsTitle) => "Configurações",
            (Self::En, Message::SettingsSubtitle) => "Make the terminal feel familiar.",
            (Self::PtBr, Message::SettingsSubtitle) => "Deixe o terminal do seu jeito.",
            (Self::En, Message::InterfaceLanguage) => "Interface language",
            (Self::PtBr, Message::InterfaceLanguage) => "Idioma da interface",
            (Self::En, Message::Automatic) => "Automatic (system)",
            (Self::PtBr, Message::Automatic) => "Automático (sistema)",
            (Self::En, Message::LanguageHint) => {
                "Changes menus and dialogs. Shell output keeps its own language."
            },
            (Self::PtBr, Message::LanguageHint) => {
                "Altera menus e telas. O shell mantém o próprio idioma."
            },
            (Self::En, Message::SaveLanguage) => "Save language",
            (Self::PtBr, Message::SaveLanguage) => "Salvar idioma",
            (Self::En, Message::LanguageSaved) => "Language saved and applied.",
            (Self::PtBr, Message::LanguageSaved) => "Idioma salvo e aplicado.",
            (Self::En, Message::LanguageSaveFailed) => {
                "Could not save the language. The previous choice was preserved."
            },
            (Self::PtBr, Message::LanguageSaveFailed) => {
                "Não foi possível salvar o idioma. A escolha anterior foi preservada."
            },
        }
    }

    #[cfg(target_os = "macos")]
    pub fn quit_summary(self, windows: usize, tabs: usize) -> String {
        let plural = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        match self {
            Self::En => {
                format!(
                    "{} in {}",
                    plural(tabs, "tab", "tabs"),
                    plural(windows, "window", "windows")
                )
            },
            Self::PtBr => {
                format!(
                    "{} em {}",
                    plural(tabs, "aba", "abas"),
                    plural(windows, "janela", "janelas")
                )
            },
        }
    }
}

#[cfg(test)]
impl Message {
    const ALL: [Self; 209] = [
        Self::AgentChats,
        Self::Applications,
        Self::PreferredEditor,
        Self::PreferredFileManager,
        Self::AppAutomatic,
        Self::SystemDefault,
        Self::AppNotInstalled,
        Self::AppSettingsHint,
        Self::OpenIn,
        Self::OpenEditor,
        Self::OpenFolder,
        Self::DesktopOpenFailed,
        Self::Agents,
        Self::AgentOverviewHint,
        Self::AgentEmpty,
        Self::AgentStartHint,
        Self::AgentIntegration,
        Self::AgentNotifications,
        Self::AgentStatusIndicator,
        Self::AgentAnimatedIndicator,
        Self::AgentStaticIndicator,
        Self::AgentSettingsHint,
        Self::AgentNewTabsHint,
        Self::AgentCodexHint,
        Self::AgentLimited,
        Self::AgentStatus,
        Self::AgentClose,
        Self::AgentReady,
        Self::AgentWorking,
        Self::AgentInput,
        Self::AgentPermission,
        Self::AgentDone,
        Self::AgentError,
        Self::AgentUnknown,
        Self::TabGroups,
        Self::GroupsHint,
        Self::GroupName,
        Self::NoGroups,
        Self::CreateGroup,
        Self::RenameGroup,
        Self::MoveToGroup,
        Self::ShowGroup,
        Self::UngroupTab,
        Self::AllTabs,
        Self::GroupInvalid,
        Self::GroupDuplicate,
        Self::MenuHeading,
        Self::NewTab,
        Self::Rename,
        Self::SaveAll,
        Self::Restore,
        Self::Copy,
        Self::Paste,
        Self::Close,
        Self::ConfirmTabClose,
        Self::CloseTabTitle,
        Self::CloseTabQuestion,
        Self::CloseTabWarning,
        Self::CloseTabAccept,
        Self::CloseTabReject,
        Self::CloseTabKeys,
        Self::Settings,
        Self::DialogTitle,
        Self::DialogSubtitle,
        Self::Name,
        Self::NameHelp,
        Self::WorkingDirectory,
        Self::DirectoryPrefix,
        Self::Unavailable,
        Self::RecentCommands,
        Self::NoCommands,
        Self::Replay,
        Self::ReplayUnavailable,
        Self::ReplayNoCommand,
        Self::ReplayHelp,
        Self::Apply,
        Self::SaveTabs,
        Self::Cancel,
        Self::RestoreShort,
        Self::KeyboardHelp,
        Self::KeyboardHelpCompact,
        Self::SavedShellFailed,
        Self::SessionReadFailed,
        Self::StateDirectoryFailed,
        Self::SessionSaved,
        Self::SessionSaveFailed,
        Self::NoSavedSession,
        Self::SessionRestoreFailed,
        Self::ShellNotReady,
        Self::CommandDirectoryMissing,
        Self::ReplayFailed,
        Self::SettingsTitle,
        Self::SettingsSubtitle,
        Self::InterfaceLanguage,
        Self::Automatic,
        Self::LanguageHint,
        Self::SaveLanguage,
        Self::LanguageSaved,
        Self::LanguageSaveFailed,
        Self::SearchForward,
        Self::SearchBackward,
        Self::TruncatedNotice,
        Self::Appearance,
        Self::AppIcon,
        Self::IconOriginal,
        Self::IconCat,
        Self::IconDog,
        Self::IconFox,
        Self::IconRobot,
        Self::IconYorkie,
        Self::IconRestore,
        Self::IconHint,
        Self::IconIntegrationFailed,
        Self::Startup,
        Self::Shortcuts,
        Self::FontFamily,
        Self::FontSize,
        Self::Opacity,
        Self::Theme,
        Self::CurrentTheme,
        Self::DarkTheme,
        Self::LightTheme,
        Self::Defaults,
        Self::RestoreAtStartup,
        Self::AutoRecovery,
        Self::StartupWorkspace,
        Self::LastSavedSession,
        Self::RecoveryHint,
        Self::DefaultShortcuts,
        Self::DuplicateTab,
        Self::ReopenClosedTab,
        Self::Workspaces,
        Self::MoveTab,
        Self::SaveSettings,
        Self::SettingsInvalid,
        Self::WorkspaceName,
        Self::NoWorkspaces,
        Self::SaveWorkspace,
        Self::OpenWorkspace,
        Self::UseAtStartup,
        Self::ConfirmDelete,
        Self::DeleteWorkspace,
        Self::WorkspaceInvalid,
        Self::PreferencesSubtitle,
        Self::WorkspacesHint,
        Self::WorkspaceSaved,
        Self::WorkspaceFailed,
        Self::WorkspaceDeleted,
        Self::StartupSaved,
        Self::NoClosedTabs,
        Self::RecoveryFailed,
        Self::GeneralPreferences,
        Self::SettingsSaved,
        Self::SettingsSaveFailed,
        Self::SavingSettings,
        Self::SavedButton,
        Self::RetryButton,
        Self::RecordShortcut,
        Self::SwapShortcuts,
        Self::ShortcutSwapHint,
        Self::Terminal,
        Self::Tabs,
        Self::Sessions,
        Self::ScrollbackLines,
        Self::HideMouseTyping,
        Self::CopyOnSelect,
        Self::Updates,
        Self::AutoCheckUpdates,
        Self::CheckUpdatesNow,
        Self::InstallUpdate,
        Self::UpdateChecking,
        Self::UpdateCurrent,
        Self::UpdateAvailable,
        Self::UpdateInstalling,
        Self::UpdateInstalled,
        Self::UpdateFailed,
        Self::UpdateUnmanaged,
        Self::UpdateNotice,
        Self::TabSettingsHint,
        Self::SettingsLanguageHint,
        Self::SplitVertical,
        Self::SplitHorizontal,
        Self::FocusPane,
        Self::PreviousTab,
        Self::NextTab,
        Self::TogglePaneZoom,
        Self::ClosePane,
        Self::PaneLimit,
        Self::ClosePaneTitle,
        Self::ClosePaneQuestion,
        Self::QuitTitle,
        Self::QuitQuestion,
        Self::QuitAccept,
        Self::ScrollbackSaveHint,
        Self::LanguagePage,
        Self::ShortcutEditHint,
        Self::ShortcutCapture,
        Self::ShortcutConflict,
        Self::ShortcutInvalid,
        Self::ShortcutDisabled,
        Self::ShortcutClear,
        Self::ShortcutChanged,
        Self::ShortcutCancelCapture,
        Self::ShortcutCurrent,
        Self::ShortcutsSubtitle,
        Self::ShortcutSearch,
        Self::ShortcutAction,
        Self::ShortcutKey,
        Self::ShortcutNoResults,
    ];
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn language_aliases_and_canonical_serialization() {
        for (aliases, expected, canonical) in [
            (&["auto", "SYSTEM"][..], Language::Auto, "auto"),
            (&["en", "en-US", "en_GB", "English"][..], Language::En, "en"),
            (&["pt", "pt-BR", "pt_PT", "Português", "PORTUGUES"][..], Language::PtBr, "pt-BR"),
        ] {
            for alias in aliases {
                let encoded = serde_json::to_string(alias).unwrap();
                let language: Language = serde_json::from_str(&encoded).unwrap();
                assert_eq!(language, expected);
                assert_eq!(serde_json::to_value(language).unwrap(), canonical);
            }
        }
    }

    #[test]
    fn general_config_preserves_explicit_language_round_trip() {
        let general: crate::config::general::General =
            toml::from_str("language = 'pt_BR'\nlive_config_reload = false").unwrap();
        assert_eq!(general.language, Language::PtBr);
        assert!(!general.live_config_reload);
        let serialized = toml::to_string(&general).unwrap();
        assert!(serialized.contains("language = \"pt-BR\""));
        let restored: crate::config::general::General = toml::from_str(&serialized).unwrap();
        assert_eq!(restored, general);
    }

    #[test]
    fn general_config_ignores_unknown_settings_and_defaults_invalid_language() {
        let unknown: crate::config::general::General =
            toml::from_str("language = 'en_GB'\nunknown_setting = true").unwrap();
        assert_eq!(unknown.language, Language::En);
        let invalid: crate::config::general::General =
            toml::from_str("language = 'private-value-123'").unwrap();
        assert_eq!(invalid.language, Language::Auto);
    }

    #[test]
    fn invalid_language_error_does_not_echo_user_value() {
        let error = serde_json::from_str::<Language>("\"private-value-123\"").unwrap_err();
        assert!(!error.to_string().contains("private-value"));
        assert!(error.to_string().contains("auto, en, or pt-BR"));
    }

    #[test]
    fn explicit_languages_and_labels() {
        assert_eq!(Language::default(), Language::Auto);
        assert_eq!(Language::En.resolve(), Locale::En);
        assert_eq!(Language::PtBr.resolve(), Locale::PtBr);
        assert_eq!(Language::En.label(Locale::PtBr), "English");
        assert_eq!(Language::PtBr.label(Locale::En), "Português (Brasil)");
        assert_ne!(Language::Auto.label(Locale::En), Language::Auto.label(Locale::PtBr));
        assert_eq!(Language::ALL.len(), 3);
    }

    #[test]
    fn locale_tags_support_regions_encodings_and_modifiers() {
        for tag in ["pt", "pt_BR.UTF-8", "pt-PT@euro", " PT_br.utf8 "] {
            assert_eq!(locale_from_tag(tag), Some(Locale::PtBr));
        }
        for tag in ["en_US.UTF-8", "en-GB", "C.UTF-8", "POSIX"] {
            assert_eq!(locale_from_tag(tag), Some(Locale::En));
        }
        for tag in ["", "fr_FR.UTF-8", "custom-locale"] {
            assert_eq!(locale_from_tag(tag), None);
        }
    }

    #[test]
    fn message_locale_precedence_and_empty_values() {
        assert_eq!(
            resolve_from(Some("en_US"), Some("pt_BR"), Some("pt_BR"), None, None),
            Locale::En
        );
        assert_eq!(resolve_from(Some(""), Some("pt_BR"), Some("en_US"), None, None), Locale::PtBr);
        assert_eq!(resolve_from(None, Some("  "), Some("pt_BR.UTF-8"), None, None), Locale::PtBr);
        assert_eq!(resolve_from(None, None, None, None, Some("pt-BR")), Locale::PtBr);
    }

    #[test]
    fn language_preference_respects_c_locale_and_supported_fallbacks() {
        assert_eq!(
            resolve_from(Some("C.UTF-8"), Some("pt_BR"), None, Some("pt_BR"), None),
            Locale::En
        );
        assert_eq!(resolve_from(None, None, Some("POSIX"), Some("pt_BR"), None), Locale::En);
        assert_eq!(
            resolve_from(None, None, Some("fr_FR"), Some("de:pt_BR:en"), None),
            Locale::PtBr
        );
        assert_eq!(resolve_from(None, None, Some("pt_BR"), Some("de:fr"), None), Locale::PtBr);
        assert_eq!(resolve_from(None, None, None, Some("pt_BR:en"), None), Locale::PtBr);
    }

    #[test]
    fn unknown_and_absent_locale_fall_back_to_english() {
        assert_eq!(resolve_from(None, None, Some("de_DE"), None, Some("pt-BR")), Locale::En);
        assert_eq!(resolve_from(None, None, None, None, None), Locale::En);
        assert_eq!(resolve_from(None, None, None, None, Some("fr-FR")), Locale::En);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quit_summary_counts_tabs_and_windows_in_both_languages() {
        assert_eq!(Locale::En.quit_summary(1, 1), "1 tab in 1 window");
        assert_eq!(Locale::En.quit_summary(2, 5), "5 tabs in 2 windows");
        assert_eq!(Locale::PtBr.quit_summary(1, 1), "1 aba em 1 janela");
        assert_eq!(Locale::PtBr.quit_summary(3, 7), "7 abas em 3 janelas");
    }

    #[test]
    fn both_catalogs_have_complete_distinct_readable_messages() {
        let mut seen = HashSet::new();
        for message in Message::ALL {
            assert!(seen.insert(format!("{message:?}")));
            for locale in [Locale::En, Locale::PtBr] {
                assert!(!locale.text(message).trim().is_empty());
                assert!(!locale.text(message).chars().any(char::is_control));
            }
            if !matches!(message, Message::Terminal | Message::IconYorkie) {
                assert_ne!(Locale::En.text(message), Locale::PtBr.text(message));
            }
        }
    }
}
