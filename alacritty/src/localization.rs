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
    extern "system" {
        fn GetUserDefaultLocaleName(locale_name: *mut u16, locale_name_count: i32) -> i32;
    }

    let mut name = [0u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    (length > 1).then(|| String::from_utf16_lossy(&name[..length as usize - 1]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
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
    Terminal,
    Tabs,
    Sessions,
    ScrollbackLines,
    HideMouseTyping,
    CopyOnSelect,
    TabSettingsHint,
    SettingsLanguageHint,
    SplitVertical,
    SplitHorizontal,
    FocusPane,
    TogglePaneZoom,
    ClosePane,
    PaneLimit,
    ClosePaneTitle,
    ClosePaneQuestion,
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
}

impl Locale {
    pub fn text(self, message: Message) -> &'static str {
        match (self, message) {
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
            (Self::En, Message::ShortcutsSubtitle) => "Customize shortcuts; changes apply on Save.",
            (Self::PtBr, Message::ShortcutsSubtitle) => "Personalize e salve seus atalhos.",

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

            (Self::En, Message::Appearance) => "Appearance",
            (Self::PtBr, Message::Appearance) => "Aparência",
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
            (Self::En, Message::AutoRecovery) => "Automatically save recovery snapshots",
            (Self::PtBr, Message::AutoRecovery) => "Salvar recuperação automaticamente",
            (Self::En, Message::StartupWorkspace) => "Startup workspace",
            (Self::PtBr, Message::StartupWorkspace) => "Ambiente inicial",
            (Self::En, Message::LastSavedSession) => "Last saved session",
            (Self::PtBr, Message::LastSavedSession) => "Última sessão salva",
            (Self::En, Message::RecoveryHint) => {
                "Recovery uses fresh shells; commands are not replayed."
            },
            (Self::PtBr, Message::RecoveryHint) => {
                "Recuperação abre shells novos sem reexecutar comandos."
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
            (Self::En, Message::KeyboardHelpCompact) => "Tab: next  Enter: confirm",
            (Self::PtBr, Message::KeyboardHelpCompact) => "Tab: próximo  Enter: confirmar",
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
}

#[cfg(test)]
impl Message {
    const ALL: [Self; 126] = [
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
        Self::Terminal,
        Self::Tabs,
        Self::Sessions,
        Self::ScrollbackLines,
        Self::HideMouseTyping,
        Self::CopyOnSelect,
        Self::TabSettingsHint,
        Self::SettingsLanguageHint,
        Self::SplitVertical,
        Self::SplitHorizontal,
        Self::FocusPane,
        Self::TogglePaneZoom,
        Self::ClosePane,
        Self::PaneLimit,
        Self::ClosePaneTitle,
        Self::ClosePaneQuestion,
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

    #[test]
    fn both_catalogs_have_complete_distinct_readable_messages() {
        let mut seen = HashSet::new();
        for message in Message::ALL {
            assert!(seen.insert(format!("{message:?}")));
            for locale in [Locale::En, Locale::PtBr] {
                assert!(!locale.text(message).trim().is_empty());
                assert!(!locale.text(message).chars().any(char::is_control));
            }
            if message != Message::Terminal {
                assert_ne!(Locale::En.text(message), Locale::PtBr.text(message));
            }
        }
    }
}
