//! Terminal Linux intégré, adossé à un vrai pseudo-terminal.
//!
//! [`PtySession`] ouvre un PTY (`portable-pty`), lance un shell dans une
//! session et un groupe de processus dédiés, puis expose l'écriture, le
//! redimensionnement et une lecture par morceaux. La lecture est volontairement
//! synchrone et bornée : [`read_chunk`] remplit le tampon fourni par l'appelant
//! (l'interface pousse ensuite les octets vers xterm.js).
//!
//! ## Scrollback et limites
//!
//! Le scrollback n'est pas stocké dans ce crate : il appartient à l'interface,
//! qui doit conserver [`scrollback_lines`] lignes (valeur de
//! `ResourcePolicy::terminal_scrollback_lines`, 5000 par défaut, plafonnée à
//! [`MAX_SCROLLBACK_LINES`]). Le PTY ne fait que transmettre les octets ; il ne
//! relit jamais l'historique. Les lectures sont bornées par le tampon de
//! l'appelant et [`READ_CHUNK_BYTES`] donne une taille de morceau raisonnable.

use ih_core::{IhError, IhResult};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use uuid::Uuid;

/// Taille de morceau conseillée pour la boucle de lecture de l'interface.
pub const READ_CHUNK_BYTES: usize = 16 * 1024;

/// Plafond absolu du scrollback, aligné sur `ih_core::ResourcePolicy`.
pub const MAX_SCROLLBACK_LINES: u32 = 100_000;

/// Nombre de lignes de scrollback à conserver côté interface.
///
/// Reprend la politique de ressources d'ih-core pour qu'une seule valeur fasse
/// foi entre le noyau, le terminal et l'interface.
pub fn scrollback_lines() -> u32 {
    ih_core::ResourcePolicy::default()
        .terminal_scrollback_lines
        .min(MAX_SCROLLBACK_LINES)
}

/// Session PTY active.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    id: String,
    exit_code: Option<i32>,
}

impl PtySession {
    /// Ouvre un PTY et lance `shell` avec `args` dans `cwd`.
    ///
    /// `env` est appliqué par-dessus l'environnement hérité ; `TERM` vaut
    /// `xterm-256color` s'il n'est fourni ni par l'appelant ni par le processus
    /// parent.
    pub fn open(
        shell: &str,
        args: &[String],
        cwd: &Path,
        cols: u16,
        rows: u16,
        env: &[(String, String)],
    ) -> IhResult<Self> {
        if shell.trim().is_empty() {
            return Err(IhError::Invalid("shell vide".to_string()));
        }
        if !cwd.is_dir() {
            return Err(IhError::Invalid(format!(
                "répertoire de travail invalide : {}",
                cwd.display()
            )));
        }
        let size = pty_size(cols, rows)?;

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(size).map_err(pty_error)?;

        let mut command = CommandBuilder::new(shell);
        command.args(args.iter().map(String::as_str));
        command.cwd(cwd);
        if command.get_env("TERM").is_none() {
            command.env("TERM", "xterm-256color");
        }
        for (key, value) in env {
            if !key.is_empty() {
                command.env(key.as_str(), value.as_str());
            }
        }

        let child = pair.slave.spawn_command(command).map_err(pty_error)?;
        // Le côté esclave ne doit vivre que dans l'enfant.
        drop(pair.slave);

        let writer = pair.master.take_writer().map_err(pty_error)?;
        Ok(Self {
            master: pair.master,
            writer,
            child,
            id: format!("pty-{}", Uuid::new_v4()),
            exit_code: None,
        })
    }

    /// Identifiant stable de la session (pour l'interface).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// PID du shell, si disponible.
    pub fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Envoie des octets au terminal (les entrées clavier, par exemple).
    pub fn write(&mut self, data: &str) -> IhResult<()> {
        self.writer.write_all(data.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    /// Redimensionne le PTY (le noyau envoie alors `SIGWINCH` au shell).
    pub fn resize(&mut self, cols: u16, rows: u16) -> IhResult<()> {
        let size = pty_size(cols, rows)?;
        self.master.resize(size).map_err(pty_error)
    }

    /// Clone le lecteur pour une boucle de lecture dédiée.
    pub fn try_clone_reader(&self) -> IhResult<Box<dyn Read + Send>> {
        self.master.try_clone_reader().map_err(pty_error)
    }

    /// Termine le shell et attend son reaping (aucun zombie conservé).
    pub fn kill(&mut self) -> IhResult<()> {
        if self.exit_code.is_some() {
            return Ok(());
        }
        match self.child.kill() {
            Ok(()) => self.wait().map(|_| ()),
            Err(error) => match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.exit_code = Some(status.exit_code() as i32);
                    Ok(())
                }
                Ok(None) => Err(IhError::Io(error)),
                // ECHILD : le processus a déjà été récolté.
                Err(_) => Ok(()),
            },
        }
    }

    /// Indique si le shell s'est terminé, sans bloquer.
    pub fn is_exited(&mut self) -> bool {
        if self.exit_code.is_some() {
            return true;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.exit_code = Some(status.exit_code() as i32);
                true
            }
            Ok(None) => false,
            Err(error) => {
                // ECHILD : le processus a déjà été récolté par un autre chemin.
                tracing::debug!("try_wait PTY en erreur : {error}");
                true
            }
        }
    }

    /// Attend la fin du shell et renvoie son code de sortie.
    pub fn wait(&mut self) -> IhResult<i32> {
        if let Some(code) = self.exit_code {
            return Ok(code);
        }
        let status = self.child.wait()?;
        let code = status.exit_code() as i32;
        self.exit_code = Some(code);
        Ok(code)
    }
}

/// Lecteur PTY asynchrone pour les commandes Tauri.
///
/// Chaque lecture bloquante est confiée au pool de tokio afin de ne pas bloquer
/// la boucle d'événements de l'interface. Le lecteur est conservé entre deux
/// lectures et le tampon est rendu à l'appelant avec le nombre d'octets lus.
pub struct AsyncPtyReader {
    reader: Option<Box<dyn Read + Send>>,
}

impl AsyncPtyReader {
    pub fn new(reader: Box<dyn Read + Send>) -> Self {
        Self {
            reader: Some(reader),
        }
    }

    /// Lit un morceau sans bloquer le runtime courant.
    pub async fn read(&mut self, mut buffer: Vec<u8>) -> IhResult<(usize, Vec<u8>)> {
        let mut reader = self.reader.take().ok_or_else(|| {
            IhError::Internal("lecteur PTY déjà consommé ou lecture en cours".to_string())
        })?;
        let task = tokio::task::spawn_blocking(move || {
            let result = read_chunk(&mut reader, &mut buffer);
            (reader, buffer, result)
        });
        let (reader, buffer, result) = task
            .await
            .map_err(|error| IhError::Internal(format!("lecture PTY interrompue : {error}")))?;
        self.reader = Some(reader);
        Ok((result?, buffer))
    }

    /// Rend le lecteur d'origine (par exemple pour revenir au mode synchrone).
    pub fn into_inner(mut self) -> IhResult<Box<dyn Read + Send>> {
        self.reader
            .take()
            .ok_or_else(|| IhError::Internal("lecteur PTY déjà consommé".to_string()))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if self.exit_code.is_none() {
            // Ne jamais laisser un shell orphelin derrière une session.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PtySession")
            .field("id", &self.id)
            .field("pid", &self.pid())
            .field("exited", &self.exit_code.is_some())
            .finish()
    }
}

/// Lit au plus `buf.len()` octets sur `reader`.
///
/// Renvoie `Ok(0)` à la fin du flux (y compris la fermeture du PTY, que Linux
/// signale par `EIO`) et réessaie sur `Interrupted`.
pub fn read_chunk(reader: &mut impl Read, buf: &mut [u8]) -> IhResult<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    loop {
        match reader.read(buf) {
            Ok(read) => return Ok(read),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                #[cfg(unix)]
                if error.raw_os_error() == Some(5) {
                    // EIO : l'esclave du PTY est fermé (shell terminé).
                    return Ok(0);
                }
                return Err(IhError::Io(error));
            }
        }
    }
}

fn pty_size(cols: u16, rows: u16) -> IhResult<PtySize> {
    if cols == 0 || rows == 0 {
        return Err(IhError::Invalid(
            "dimensions du terminal invalides (colonnes et lignes doivent être > 0)".to_string(),
        ));
    }
    Ok(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })
}

fn pty_error(error: impl std::fmt::Display) -> IhError {
    IhError::Internal(format!("pty : {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::thread;
    use std::time::{Duration, Instant};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("ih-workspace-pty-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("création du dossier temporaire");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Démarre une boucle de lecture qui pousse les octets bruts au test.
    fn spawn_reader(session: &PtySession) -> (Receiver<Vec<u8>>, thread::JoinHandle<()>) {
        let mut reader = session.try_clone_reader().expect("clone du lecteur PTY");
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut buffer = [0u8; READ_CHUNK_BYTES];
            loop {
                match read_chunk(&mut reader, &mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        if sender.send(buffer[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        eprintln!("lecture PTY terminée : {error}");
                        break;
                    }
                }
            }
        });
        (receiver, handle)
    }

    /// Supprime les séquences d'échappement ANSI (CSI et OSC).
    ///
    /// Un PTY réel émet des séquences de contrôle — par exemple le bracketed
    /// paste `ESC[?2004h` — que le shell n'affiche jamais textuellement.
    fn strip_ansi(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        while let Some(current) = chars.next() {
            if current != '\u{1b}' {
                out.push(current);
                continue;
            }
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    // CSI : paramètres puis octet final dans 0x40..=0x7E.
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    // OSC : jusqu'à BEL ou ESC \.
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Normalise une ligne de PTY : retire ANSI, CR et le prompt éventuel
    /// (`$ hello`, `bash-5.3# hello`).
    fn without_prompt(line: &str) -> String {
        let cleaned = strip_ansi(line);
        let cleaned = cleaned.trim_matches(|c: char| c == '\r' || c.is_whitespace());
        let cleaned = cleaned.rsplit("$ ").next().unwrap_or(cleaned);
        let cleaned = cleaned.rsplit("# ").next().unwrap_or(cleaned);
        cleaned
            .trim_matches(|c: char| c == '\r' || c.is_whitespace())
            .to_string()
    }

    fn wait_for_line(receiver: &Receiver<Vec<u8>>, expected: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        let mut output = String::new();
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(remaining) {
                Ok(chunk) => {
                    output.push_str(&String::from_utf8_lossy(&chunk));
                    if output.lines().any(|line| without_prompt(line) == expected) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        output
    }

    #[test]
    fn normalisation_de_la_sortie_reelle_d_un_pty() {
        // Sorties réellement capturées dans la CI Arch et Ubuntu.
        let arch = "echo hello\r\n\u{1b}[?2004hsh-5.3# echo hello\r\n\u{1b}[?2004l\rhello\r\n\u{1b}[?2004hsh-5.3# ";
        let bash = "printf 'bash_ok\\n'\r\n\u{1b}[?2004hbash-5.3# printf 'bash_ok\\n'\r\n\u{1b}[?2004l\rbash_ok\r\ncwd=/tmp/x\r\nenv=valeur-ok\r\n";
        for (raw, expected) in [(arch, "hello"), (bash, "bash_ok"), (bash, "env=valeur-ok")] {
            assert!(
                raw.lines().any(|line| without_prompt(line) == expected),
                "sortie: {raw:?}"
            );
        }
    }

    #[test]
    fn pty_sh_echo_hello_puis_arret_sans_zombie() {
        let cwd = TempDir::new();
        let mut session =
            PtySession::open("/bin/sh", &[], cwd.path(), 80, 24, &[]).expect("ouverture PTY");
        assert!(session.id().starts_with("pty-"));
        assert!(!session.id().is_empty());

        let pid = session.pid();
        let (receiver, reader) = spawn_reader(&session);
        session.write("echo hello\n").expect("écriture PTY");

        let output = wait_for_line(&receiver, "hello", Duration::from_secs(10));
        assert!(
            output.lines().any(|line| without_prompt(line) == "hello"),
            "sortie PTY inattendue : {output:?}"
        );

        session.kill().expect("arrêt du shell");
        assert!(session.is_exited());
        let code = session.wait().expect("attente du shell");
        let _ = code;
        reader.join().expect("boucle de lecture terminée");

        if let Some(pid) = pid {
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "le processus {pid} aurait dû être récolté"
            );
        }
    }

    #[test]
    fn pty_bash_respecte_cwd_et_env() {
        assert!(Path::new("/bin/bash").is_file(), "/bin/bash est requis");
        let cwd = TempDir::new();
        let env = vec![("IH_PTY_TEST".to_string(), "valeur-ok".to_string())];
        let mut session = PtySession::open(
            "/bin/bash",
            &["--norc".to_string(), "--noprofile".to_string()],
            cwd.path(),
            100,
            30,
            &env,
        )
        .expect("ouverture PTY bash");

        let (receiver, reader) = spawn_reader(&session);
        session
            .write("printf 'bash_ok\\n'; printf 'cwd=%s\\n' \"$PWD\"; printf 'env=%s\\n' \"$IH_PTY_TEST\"\n")
            .expect("écriture PTY");

        let output = wait_for_line(&receiver, "env=valeur-ok", Duration::from_secs(10));
        assert!(
            output.lines().any(|line| without_prompt(line) == "bash_ok"),
            "bash n'a pas répondu : {output:?}"
        );
        assert!(
            output
                .lines()
                .any(|line| without_prompt(line) == "env=valeur-ok"),
            "environnement non transmis : {output:?}"
        );
        let expected_cwd = format!(
            "cwd={}",
            cwd.path()
                .canonicalize()
                .unwrap_or_else(|_| cwd.path().to_path_buf())
                .display()
        );
        assert!(
            output
                .lines()
                .any(|line| without_prompt(line) == expected_cwd),
            "répertoire courant non appliqué : {output:?}"
        );

        session.kill().expect("arrêt de bash");
        reader.join().expect("boucle de lecture terminée");
    }

    #[test]
    fn resize_sans_erreur() {
        let cwd = TempDir::new();
        let mut session =
            PtySession::open("/bin/sh", &[], cwd.path(), 80, 24, &[]).expect("ouverture PTY");
        session.resize(120, 40).expect("agrandissement");
        session.resize(40, 12).expect("rétrécissement");
        session.kill().expect("arrêt");
        assert!(session.is_exited());
    }

    #[tokio::test]
    async fn lecture_asynchrone_bornee() {
        let mut reader = AsyncPtyReader::new(Box::new(std::io::Cursor::new(b"async".to_vec())));
        let (read, buffer) = reader.read(vec![0u8; 8]).await.unwrap();
        assert_eq!(read, 5);
        assert_eq!(&buffer[..read], b"async");

        let (read, _) = reader.read(vec![0u8; 8]).await.unwrap();
        assert_eq!(read, 0);
        let mut inner = reader.into_inner().expect("lecteur toujours disponible");
        let mut buffer = [0u8; 4];
        assert_eq!(read_chunk(&mut inner, &mut buffer).unwrap(), 0);
    }

    #[test]
    fn read_chunk_borne_et_fin_de_flux() {
        let mut cursor = std::io::Cursor::new(b"abc".to_vec());
        let mut buffer = [0u8; 8];
        assert_eq!(read_chunk(&mut cursor, &mut buffer).unwrap(), 3);
        assert_eq!(read_chunk(&mut cursor, &mut buffer).unwrap(), 0);

        let mut empty = std::io::Cursor::new(Vec::<u8>::new());
        assert_eq!(read_chunk(&mut empty, &mut buffer).unwrap(), 0);
        assert_eq!(read_chunk(&mut empty, &mut []).unwrap(), 0);
    }

    #[test]
    fn scrollback_aligne_sur_ih_core() {
        let lines = scrollback_lines();
        assert_eq!(
            lines,
            ih_core::ResourcePolicy::default().terminal_scrollback_lines
        );
        assert!(lines > 0);
        assert!(lines <= MAX_SCROLLBACK_LINES);
    }
}
