//! A deterministic simulation of iglud and hostd together. Both run for
//! real, the API, the reconciler, the database, hostd's rules and the HTTP
//! between them, over hostd's simulated runtime, in one process. A seeded
//! driver chooses each next event: a person's request, a reconciler pass,
//! the end of a step a host is taking, a fault, or time passing. It checks
//! invariants after every event and, once the faults stop, that everything
//! converges on what people asked for.
//!
//! Every step a host takes waits in the simulated runtime until the driver
//! resolves it, time moves only when the driver moves it, and the driver
//! waits for everything else to settle before choosing again. So the driver
//! alone decides the order of everything that matters, and a seed replays.
//!
//! `cargo test -p iglud sim` runs a few schedules. `IGLU_SIM_SCHEDULES`,
//! `IGLU_SIM_STEPS` and `IGLU_SIM_SEED` run more, longer or from a seed,
//! which replays a failure.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant as WallClock};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hyper_util::rt::TokioIo;
use iglu_domain::auth::VerifiedIdentity;
use iglu_domain::id::{InstanceName, PrincipalId, WorkspaceId};
use iglu_domain::lifecycle::{DesiredState, Instance, Present, Runtime as Status};
use iglu_domain::time::Millis;
use iglu_hostd::config::Timeouts;
use iglu_hostd::host::Host;
use iglu_hostd::sim::{Fault, Origin, Pending, Power, SimConfig, SimRuntime};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tower::ServiceExt;
use uuid::Uuid;

use crate::app::{App, ColumnEdits, now};
use crate::config::Config;
use crate::crypto::{self, Sealer};
use crate::db::{self, Db, SessionKind};
use crate::hosts::HostClient;
use crate::model::Condition;
use crate::oidc::{Provider, RelyingParty};

const CONSOLE_HOST: &str = "iglu.sim.test";
const REPO: &str = "https://git.sim.test/app.git";
const PEOPLE: usize = 2;
const MAX_WORKSPACES: usize = 4;
/// Longer than any backoff and any request's timeout, so a quiet stretch
/// of it lets everything due happen.
const QUIET: Duration = Duration::from_secs(301);

/// `SplitMix64`: every choice the driver makes comes from it.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n).expect("a count fits in u64");
        usize::try_from(self.next() % n).expect("below a usize")
    }

    fn percent(&mut self, chance: usize) -> bool {
        self.below(100) < chance
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        (!items.is_empty()).then(|| &items[self.below(items.len())])
    }
}

/// A connection to hostd that a process's death can hang: what its other
/// end was waiting for never comes, and nothing it sends arrives.
struct Line {
    stream: TcpStream,
    hung: Arc<AtomicBool>,
}

impl AsyncRead for Line {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.hung.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for Line {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.hung.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.hung.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.hung.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

type Connections = Arc<Mutex<Vec<(JoinHandle<()>, Arc<AtomicBool>)>>>;

/// hostd, serving its real routes on loopback over the simulated runtime.
/// Each run of iglud reaches it through a listener of its own, so when that
/// iglud dies, every connection it has or makes later can hang with it.
struct Hostd {
    router: Arc<Mutex<axum::Router>>,
    connections: Connections,
    /// Whether each run of iglud has died, by listener.
    dead: Vec<Arc<AtomicBool>>,
}

impl Hostd {
    fn new(runtime: &SimRuntime) -> Self {
        Self {
            router: Arc::new(Mutex::new(hostd_router(runtime))),
            connections: Connections::default(),
            dead: Vec::new(),
        }
    }

    /// A listener for the next run of iglud.
    async fn listen(&mut self) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port is free");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        let dead = Arc::new(AtomicBool::new(false));
        self.dead.push(dead.clone());
        tokio::spawn(accept(
            listener,
            self.router.clone(),
            self.connections.clone(),
            dead,
        ));
        address
    }

    /// hostd restarts: its connections close, and it comes back with
    /// nothing in memory.
    fn restart(&self, runtime: &SimRuntime) {
        lock(&self.connections).retain(|(task, dead)| {
            let hung = dead.load(Ordering::SeqCst);
            if !hung {
                task.abort();
            }
            hung
        });
        *lock(&self.router) = hostd_router(runtime);
    }

    /// The current run of iglud dies: whatever it waits for from hostd, now
    /// or later, never comes.
    fn bury(&self) {
        if let Some(dead) = self.dead.last() {
            dead.store(true, Ordering::SeqCst);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .expect("the simulation's locks are never held across a panic")
}

fn hostd_router(runtime: &SimRuntime) -> axum::Router {
    iglu_hostd::server::router(Arc::new(iglu_hostd::server::App {
        host_id: "sim".parse().expect("a valid host id"),
        arch: iglu_hostd::host_arch().expect("a supported architecture"),
        host: Host::new(runtime.clone(), Timeouts::default()),
    }))
}

async fn accept(
    listener: TcpListener,
    router: Arc<Mutex<axum::Router>>,
    connections: Connections,
    dead: Arc<AtomicBool>,
) {
    while let Ok((stream, _)) = listener.accept().await {
        let router = lock(&router).clone();
        let service = hyper::service::service_fn(move |request| {
            iglu_hostd::sim::serving(router.clone().oneshot(request))
        });
        let line = Line {
            stream,
            hung: dead.clone(),
        };
        let task = tokio::spawn(async move {
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(line), service)
                .await;
        });
        lock(&connections).push((task, dead.clone()));
    }
}

/// iglud, built as `main` builds it, but for its listener and TLS.
struct Iglud {
    app: Arc<App>,
    router: axum::Router,
}

fn write_config(dir: &Path, hostd: SocketAddr) {
    std::fs::write(dir.join("secret-key"), "11".repeat(32)).expect("written");
    let config = json!({
        "listen": "127.0.0.1:0",
        "console_origin": format!("https://{CONSOLE_HOST}"),
        "preview_domain": "preview.sim.test",
        "database": dir.join("iglu.db"),
        "secret_key_file": dir.join("secret-key"),
        "console_assets": dir,
        "oidc": {
            "issuer": "https://idp.sim.test",
            "console": {"client_id": "console", "client_secret_file": dir.join("unused")},
            "preview": {"client_id": "preview", "client_secret_file": dir.join("unused")},
        },
        "sign_in": (0..PEOPLE)
            .map(|i| json!({"verified_email": format!("person-{i}@example.org")}))
            .collect::<Vec<_>>(),
        "hosts": [{"id": "sim", "url": format!("http://{hostd}")}],
        "host_client": {"certificate": dir.join("unused"), "key": dir.join("unused")},
        // Idleness reads the wall clock, which a simulation doesn't move.
        "idle": {"freeze_after_minutes": null, "stop_after_minutes": null},
    });
    std::fs::write(dir.join("iglud.json"), config.to_string()).expect("written");
}

fn start_iglud(dir: &Path) -> Iglud {
    let config = Config::load(&dir.join("iglud.json")).expect("a valid configuration");
    let db = Db::open(&config.database).expect("the database opens");
    let sealer = Sealer::load(&config.secret_key_file).expect("the key loads");
    // Everything it calls is plain HTTP on loopback, so it trusts no CA,
    // and needs no system store, which a build sandbox doesn't have.
    let tls = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth(),
    );
    let http = reqwest::Client::builder()
        .use_preconfigured_tls((*tls).clone())
        .timeout(Duration::from_secs(4000))
        .build()
        .expect("an HTTP client");
    let provider = Arc::new(Provider::new(http.clone(), &config.oidc.issuer).expect("an issuer"));
    let callback = |path: &str| {
        config
            .console_origin
            .join(path)
            .expect("a callback under the console")
    };
    let console = RelyingParty::new(
        provider.clone(),
        "console".into(),
        "unused".into(),
        callback("/auth/callback"),
    );
    let preview = RelyingParty::new(
        provider,
        "preview".into(),
        "unused".into(),
        callback("/preview/callback"),
    );
    let hosts = config
        .hosts
        .iter()
        .map(|host| {
            Arc::new(HostClient::new(
                host.id.clone(),
                host.url.as_url().clone(),
                http.clone(),
                tls.clone(),
            ))
        })
        .collect();
    let (changes, _) = tokio::sync::broadcast::channel(64);
    let app = Arc::new(App {
        config,
        db,
        sealer,
        console,
        preview,
        hosts,
        changes,
        host_seen: Mutex::default(),
        reconciler: Arc::default(),
        pool: crate::gateway::Pool::default(),
        usage: crate::idle::Usage::new(now()),
        boot: iglu_api::BootId::from_uuid(Uuid::new_v4()),
        column_edits: ColumnEdits::default(),
        uploads: tokio::sync::Semaphore::new(4),
    });
    let router = crate::router(app.clone(), dir);
    Iglud { app, router }
}

/// Sends a request to iglud's API as a person, in process.
async fn call(
    router: axum::Router,
    token: String,
    method: Method,
    path: String,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, CONSOLE_HOST)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json");
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = router
        .oneshot(request.body(body).expect("a valid request"))
        .await
        .expect("a router always answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

struct Person {
    principal: PrincipalId,
    token: String,
    project: Value,
    secrets: u32,
}

struct Workspace {
    id: WorkspaceId,
    owner: usize,
}

/// A request a person made that hasn't been answered.
struct Asked {
    task: JoinHandle<(StatusCode, Value)>,
    what: String,
    workspace: Option<usize>,
    /// What to do with the answer.
    then: Then,
}

#[derive(Clone, Copy)]
enum Then {
    Nothing,
    Created(usize),
    Column(usize),
}

/// What a schedule found, with the trace that led to it.
pub struct Report {
    pub findings: Vec<String>,
    /// Steps refused because hostd acted on an instance changing state.
    pub refused_while_changing: usize,
    pub digest: String,
    pub trace: Vec<String>,
}

struct Sim {
    rng: Rng,
    dir: PathBuf,
    runtime: SimRuntime,
    hostd: Hostd,
    iglud: Iglud,
    people: Vec<Person>,
    workspaces: Vec<Workspace>,
    columns: Vec<Vec<String>>,
    asked: Vec<Asked>,
    /// Builds an iglud that died had under way, which never finish.
    dead: BTreeSet<String>,
    trace: Vec<String>,
    findings: Vec<String>,
    owners: HashMap<PrincipalId, usize>,
    violations_seen: usize,
    /// The steps pending when the last pass looked, which it saw as
    /// changing their instances' state.
    before_pass: BTreeSet<u64>,
}

impl Sim {
    async fn new(seed: u64, dir: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("c")).expect("made");
        let runtime = SimRuntime::new(SimConfig {
            gated: true,
            repos: BTreeSet::from([REPO.to_owned()]),
            channels: dir.join("c"),
            operation: Timeouts::default().operation(),
        });
        let mut hostd = Hostd::new(&runtime);
        write_config(&dir, hostd.listen().await);
        let iglud = start_iglud(&dir);
        let mut sim = Self {
            rng: Rng(seed),
            dir,
            runtime,
            hostd,
            iglud,
            people: Vec::new(),
            workspaces: Vec::new(),
            columns: Vec::new(),
            asked: Vec::new(),
            dead: BTreeSet::new(),
            trace: Vec::new(),
            findings: Vec::new(),
            owners: HashMap::new(),
            violations_seen: 0,
            before_pass: BTreeSet::new(),
        };
        sim.set_up().await;
        sim
    }

    /// Signs everyone in, and gives each an environment, built, a project
    /// with a repository, and a secret, through the API.
    async fn set_up(&mut self) {
        for i in 0..PEOPLE {
            let issuer = self.iglud.app.config.oidc.issuer.clone();
            let identity: VerifiedIdentity = VerifiedIdentity {
                issuer,
                subject: format!("person-{i}").parse().expect("a subject"),
                email: Some(format!("person-{i}@example.org").parse().expect("an email")),
                name: None,
            };
            let token = crypto::token().expect("randomness");
            let hash = crypto::hash(&token);
            let principal = self
                .iglud
                .app
                .db
                .call(move |tx| {
                    let principal = db::upsert_principal(
                        tx,
                        PrincipalId::from_uuid(Uuid::new_v4()),
                        &identity,
                        now(),
                    )?;
                    let at = now();
                    db::create_session(
                        tx,
                        &hash,
                        SessionKind::Api,
                        principal.id,
                        at,
                        at.plus(Millis::from_secs(365 * 86_400)),
                    )?;
                    Ok(principal.id)
                })
                .await
                .expect("signing in");
            self.owners.insert(principal, i);
            self.people.push(Person {
                principal,
                token,
                project: Value::Null,
                secrets: 0,
            });
            let source = format!("path:/sim#person-{i}");
            self.must(
                i,
                Method::POST,
                "/v1/environments",
                json!({"name": "env", "source": source}),
            )
            .await;
        }
        // Adding an environment builds it.
        for i in 0..PEOPLE {
            self.change_secret(i);
        }
        self.settle().await;
        while let Some(build) = self
            .pending()
            .into_iter()
            .find(|p| p.origin == Origin::Build)
        {
            self.runtime.resolve(build.seq, Fault::None);
            self.settle().await;
        }
        self.harvest();
        for i in 0..PEOPLE {
            let (status, project) = self
                .must(
                    i,
                    Method::POST,
                    "/v1/projects",
                    json!({"name": "app", "repo": REPO, "environment": "env"}),
                )
                .await;
            assert_eq!(status, StatusCode::CREATED, "a project: {project}");
            self.people[i].project = project["id"].clone();
        }
        self.settle().await;
        self.harvest();
        self.trace.clear();
    }

    /// A request answered before the simulation goes on, for setting up.
    async fn must(
        &self,
        person: usize,
        method: Method,
        path: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        call(
            self.iglud.router.clone(),
            self.people[person].token.clone(),
            method,
            path.to_owned(),
            Some(body),
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "each says what's asked, and what follows"
    )]
    fn ask(
        &mut self,
        person: usize,
        workspace: Option<usize>,
        what: &str,
        method: Method,
        path: String,
        body: Option<Value>,
        then: Then,
    ) {
        let task = tokio::spawn(call(
            self.iglud.router.clone(),
            self.people[person].token.clone(),
            method,
            path,
            body,
        ));
        self.asked.push(Asked {
            task,
            what: what.to_owned(),
            workspace,
            then,
        });
    }

    fn change_secret(&mut self, person: usize) {
        self.people[person].secrets += 1;
        let value = format!("person-{person}-secret-{}", self.people[person].secrets);
        let body = json!({"target": {"kind": "env", "name": "TOKEN"}, "value": value});
        self.ask(
            person,
            None,
            "secret",
            Method::PUT,
            "/v1/secrets/token".into(),
            Some(body),
            Then::Nothing,
        );
    }

    /// The pending steps, in an order that doesn't depend on when their
    /// callers happened to register them.
    fn pending(&self) -> Vec<Pending> {
        let mut pending = self.runtime.pending();
        let index = |name: Option<InstanceName>| {
            name.and_then(|n| {
                self.workspaces
                    .iter()
                    .position(|w| w.id.instance_name() == n)
            })
            .unwrap_or(usize::MAX)
        };
        pending.sort_by_key(|p| {
            (
                index(p.instance),
                p.source.as_ref().map(ToString::to_string),
                p.origin,
                p.what,
                p.orphaned,
                p.seq,
            )
        });
        pending
    }

    fn describe(&self, pending: &Pending) -> String {
        let at = match (pending.instance, &pending.source) {
            (Some(name), _) => self
                .workspaces
                .iter()
                .position(|w| w.id.instance_name() == name)
                .map_or_else(|| "a stranger".into(), |i| format!("w{i}")),
            (None, Some(source)) => source.to_string(),
            (None, None) => "nothing".into(),
        };
        let orphan = if pending.orphaned { " (orphaned)" } else { "" };
        format!("{} at {at}{orphan}", pending.what)
    }

    /// Waits until everything that can happen without the driver has: each
    /// effect, request and build under way is waiting on a pending step.
    async fn settle(&mut self) -> bool {
        for _ in 0..200_000 {
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            if self.quiet().await {
                return true;
            }
        }
        let pending: Vec<String> = self.pending().iter().map(|p| self.describe(p)).collect();
        self.find(format!(
            "stuck: {} effects, {} requests and {} builds under way, pending {pending:?}",
            self.iglud.app.reconciler.in_flight().len(),
            self.asked.iter().filter(|a| !a.task.is_finished()).count(),
            self.building().await,
        ));
        false
    }

    /// Whether everything under way waits on a pending step. A step whose
    /// caller gave up, as iglud does when a request times out, waits on
    /// nobody's behalf: it's marked orphaned.
    async fn quiet(&mut self) -> bool {
        let claimed: Vec<InstanceName> = self
            .iglud
            .app
            .reconciler
            .in_flight()
            .iter()
            .map(|id| id.instance_name())
            .collect();
        let asking: Vec<InstanceName> = self
            .asked
            .iter()
            .filter(|a| !a.task.is_finished())
            .filter_map(|a| a.workspace.map(|w| self.workspaces[w].id.instance_name()))
            .collect();
        let mut builds = Vec::new();
        let (mut effects, mut requests) = (0, 0);
        for pending in self.runtime.pending().into_iter().filter(|p| !p.orphaned) {
            let waited = |by: &[InstanceName]| pending.instance.is_some_and(|i| by.contains(&i));
            match pending.origin {
                // The reconciler claims a workspace before it starts an
                // effect, and lets go when the effect's answer comes or
                // doesn't.
                Origin::Effect if !waited(&claimed) => self.runtime.orphan_step(pending.seq),
                Origin::Request if !waited(&asking) => self.runtime.orphan_step(pending.seq),
                Origin::Effect => effects += 1,
                Origin::Request => requests += 1,
                Origin::Build => builds.push(pending.seq),
            }
        }
        if effects != claimed.len()
            || requests != self.asked.iter().filter(|a| !a.task.is_finished()).count()
        {
            return false;
        }
        let building = self.building().await;
        // A build iglud stopped waiting for still finishes on the host.
        builds.sort_unstable();
        while builds.len() > building {
            self.runtime.orphan_step(builds.remove(0));
        }
        builds.len() == building
    }

    /// Builds under way, but for those an iglud that died left behind.
    async fn building(&self) -> usize {
        self.building_ids()
            .await
            .iter()
            .filter(|id| !self.dead.contains(*id))
            .count()
    }

    async fn building_ids(&self) -> Vec<String> {
        self.iglud
            .app
            .db
            .call(|tx| {
                let mut statement =
                    tx.prepare("SELECT id FROM env_revision WHERE status = 'building'")?;
                let ids = statement
                    .query_map([], |row| row.get(0))?
                    .collect::<Result<Vec<String>, _>>()?;
                Ok(ids)
            })
            .await
            .expect("reading builds")
    }

    /// Takes the answers to requests, and notes what they made.
    fn harvest(&mut self) {
        let mut answered = Vec::new();
        self.asked.retain_mut(|asked| {
            if asked.task.is_finished() {
                answered.push((
                    std::mem::replace(
                        &mut asked.task,
                        tokio::spawn(async { (StatusCode::OK, Value::Null) }),
                    ),
                    asked.what.clone(),
                    asked.workspace,
                    asked.then,
                ));
                false
            } else {
                true
            }
        });
        for (task, what, workspace, then) in answered {
            let Some(Ok((status, body))) = futures_util::FutureExt::now_or_never(task) else {
                continue;
            };
            let at = workspace.map_or_else(String::new, |w| format!(" w{w}"));
            self.trace
                .push(format!("  answered {what}{at}: {}", status.as_u16()));
            match then {
                Then::Created(owner) if status == StatusCode::ACCEPTED => {
                    if let Some(id) = body["id"].as_str().and_then(|id| id.parse().ok()) {
                        self.workspaces.push(Workspace { id, owner });
                        self.columns.push(Vec::new());
                    }
                }
                Then::Column(w) if status == StatusCode::CREATED => {
                    if let Some(name) = body["name"].as_str() {
                        self.columns[w].push(name.to_owned());
                    }
                }
                Then::Nothing | Then::Created(_) | Then::Column(_) => {}
            }
        }
    }

    fn find(&mut self, finding: String) {
        self.trace.push(format!("FINDING {finding}"));
        self.findings.push(finding);
    }

    /// Whether a request about the workspace is still waiting, so another
    /// would only queue behind it.
    fn asking_about(&self, workspace: usize) -> bool {
        self.asked
            .iter()
            .any(|a| a.workspace == Some(workspace) && !a.task.is_finished())
    }

    async fn step(&mut self) {
        let roll = self.rng.below(100);
        let description = match roll {
            0..12 => self.create(),
            12..26 => self.desire(),
            26..30 => self.update(),
            30..33 => self.rebuild(),
            33..37 => {
                let person = self.rng.below(PEOPLE);
                self.change_secret(person);
                format!("p{person} changes a secret")
            }
            37..43 => self.add_column(),
            43..46 => self.close_column(),
            46..64 => {
                self.before_pass = self.runtime.pending().iter().map(|p| p.seq).collect();
                crate::reconcile::pass(&self.iglud.app, &self.iglud.app.reconciler).await;
                "a reconciler pass".into()
            }
            64..86 => self.resolve(),
            86..92 => {
                let seconds = *self.rng.pick(&[1, 10, 60, 301]).expect("not empty");
                tokio::time::advance(Duration::from_secs(seconds)).await;
                format!("{seconds}s pass")
            }
            92..94 => {
                self.runtime.reboot();
                self.hostd.restart(&self.runtime);
                "the host restarts".into()
            }
            94..96 => self.crash(),
            96..98 => {
                self.hostd.restart(&self.runtime);
                self.orphan_all();
                "hostd restarts".into()
            }
            _ => self.restart_iglud().await,
        };
        self.trace.push(description);
        if self.settle().await {
            self.harvest();
            self.check((46..64).contains(&roll)).await;
        }
    }

    fn orphan_all(&self) {
        for origin in [Origin::Effect, Origin::Request, Origin::Build] {
            self.runtime.orphan(origin);
        }
    }

    async fn restart_iglud(&mut self) -> String {
        self.hostd.bury();
        for asked in self.asked.drain(..) {
            asked.task.abort();
        }
        self.orphan_all();
        self.dead.extend(self.building_ids().await);
        write_config(&self.dir, self.hostd.listen().await);
        self.iglud = start_iglud(&self.dir);
        "iglud restarts".into()
    }

    fn create(&mut self) -> String {
        if self.workspaces.len() >= MAX_WORKSPACES {
            return self.resolve();
        }
        let person = self.rng.below(PEOPLE);
        let body = json!({"project": self.people[person].project});
        self.ask(
            person,
            None,
            "create",
            Method::POST,
            "/v1/workspaces".into(),
            Some(body),
            Then::Created(person),
        );
        format!("p{person} creates a workspace")
    }

    fn workspace(&mut self) -> Option<usize> {
        (!self.workspaces.is_empty()).then(|| self.rng.below(self.workspaces.len()))
    }

    fn desire(&mut self) -> String {
        let Some(w) = self.workspace() else {
            return self.create();
        };
        let state = *self
            .rng
            .pick(&[
                DesiredState::Running,
                DesiredState::Running,
                DesiredState::Frozen,
                DesiredState::Stopped,
                DesiredState::Deleted,
            ])
            .expect("not empty");
        let owner = self.workspaces[w].owner;
        let path = format!("/v1/workspaces/{}/desired-state", self.workspaces[w].id);
        let body = serde_json::to_value(iglu_api::SetDesiredState { state }).expect("serializes");
        self.ask(
            owner,
            Some(w),
            "desire",
            Method::PUT,
            path,
            Some(body),
            Then::Nothing,
        );
        format!("w{w} is asked to be {state:?}")
    }

    fn update(&mut self) -> String {
        let Some(w) = self.workspace() else {
            return self.create();
        };
        let owner = self.workspaces[w].owner;
        let path = format!("/v1/workspaces/{}/update", self.workspaces[w].id);
        self.ask(
            owner,
            Some(w),
            "update",
            Method::POST,
            path,
            None,
            Then::Nothing,
        );
        format!("w{w} is asked to update")
    }

    fn rebuild(&mut self) -> String {
        let person = self.rng.below(PEOPLE);
        self.ask(
            person,
            None,
            "build",
            Method::POST,
            "/v1/environments/env/builds".into(),
            None,
            Then::Nothing,
        );
        format!("p{person} rebuilds their environment")
    }

    fn add_column(&mut self) -> String {
        let Some(w) = self.workspace() else {
            return self.create();
        };
        if self.asking_about(w) {
            return self.resolve();
        }
        let owner = self.workspaces[w].owner;
        let path = format!("/v1/workspaces/{}/columns", self.workspaces[w].id);
        let body = json!({"kind": {"kind": "shell"}});
        self.ask(
            owner,
            Some(w),
            "add a column",
            Method::POST,
            path,
            Some(body),
            Then::Column(w),
        );
        format!("w{w} is asked for a column")
    }

    fn close_column(&mut self) -> String {
        let Some(w) = self.workspace() else {
            return self.create();
        };
        if self.asking_about(w) || self.columns[w].is_empty() {
            return self.resolve();
        }
        let index = self.rng.below(self.columns[w].len());
        let name = self.columns[w].remove(index);
        let owner = self.workspaces[w].owner;
        let path = format!("/v1/workspaces/{}/columns/{name}", self.workspaces[w].id);
        self.ask(
            owner,
            Some(w),
            "close a column",
            Method::DELETE,
            path,
            None,
            Then::Nothing,
        );
        format!("w{w} is asked to close a column")
    }

    fn resolve(&mut self) -> String {
        let pending = self.pending();
        let Some(chosen) = self.rng.pick(&pending).cloned() else {
            return "nothing is pending".into();
        };
        let fault = match self.rng.below(10) {
            0 => Fault::Before,
            1 => Fault::After,
            _ => Fault::None,
        };
        self.runtime.resolve(chosen.seq, fault);
        format!("{} ends: {fault:?}", self.describe(&chosen))
    }

    fn crash(&mut self) -> String {
        let held = self.runtime.instances_held();
        let Some((name, _)) = self.rng.pick(&held).cloned() else {
            return self.resolve();
        };
        let failed = self.rng.percent(50);
        self.runtime.crash(name, failed);
        let at = self
            .workspaces
            .iter()
            .position(|w| w.id.instance_name() == name)
            .map_or_else(|| "a stranger".into(), |i| format!("w{i}"));
        format!("{at} crashes{}", if failed { " into failure" } else { "" })
    }

    /// What must hold after every event.
    async fn check(&mut self, after_pass: bool) {
        let violations = self.runtime.violations();
        for violation in &violations[self.violations_seen..] {
            self.findings.push(format!(
                "hostd asked for a step the state forbids: {violation}"
            ));
        }
        self.violations_seen = violations.len();
        for delivery in self.runtime.take_deliveries() {
            let owner = self.owners.get(&delivery.owner).copied();
            let foreign: Vec<&String> = delivery
                .values
                .iter()
                .filter(|v| owner.is_none_or(|o| !v.starts_with(&format!("person-{o}-"))))
                .collect();
            if !foreign.is_empty() {
                self.find(format!("a guest of person {owner:?} was given {foreign:?}"));
            }
            if delivery.replaced > Some(delivery.generation) {
                self.find(format!(
                    "a guest's secrets went back from {:?} to {:?} within a boot",
                    delivery.replaced, delivery.generation
                ));
            }
        }
        if after_pass {
            self.check_observations().await;
        }
        let state = self.state().await;
        self.trace.push(format!("  = {state}"));
    }

    /// After a pass, iglud's record of each live workspace's instance is
    /// what the host showed it.
    async fn check_observations(&mut self) {
        let records = self
            .iglud
            .app
            .db
            .call(|tx| db::live_workspaces(tx))
            .await
            .expect("reading workspaces");
        let held = self.runtime.instances_held();
        let pending: Vec<Pending> = self
            .runtime
            .pending()
            .into_iter()
            .filter(|p| self.before_pass.contains(&p.seq))
            .collect();
        for record in records {
            // A pass that couldn't reach hostd records nothing it saw.
            if matches!(record.condition, Some(Condition::HostOffline { .. })) {
                self.trace.push(format!(
                    "  the pass couldn't reach hostd for w{}",
                    self.index(record.id)
                ));
                continue;
            }
            let name = record.id.instance_name();
            let changing = pending.iter().any(|p| {
                p.instance == Some(name)
                    && p.origin == Origin::Effect
                    && LIFECYCLE.contains(&p.what)
            });
            let shown = match held.iter().find(|(n, _)| *n == name) {
                None => "absent",
                Some(_) if changing => "transitioning",
                Some((_, instance)) => power(instance.power),
            };
            let recorded = record.observed.map_or("absent", kind);
            if shown != recorded {
                let w = self.index(record.id);
                self.find(format!(
                    "after a pass, iglud records w{w} as {recorded} while the host shows {shown}"
                ));
            }
        }
    }

    fn index(&self, id: WorkspaceId) -> String {
        self.workspaces
            .iter()
            .position(|w| w.id == id)
            .map_or_else(|| "?".into(), |i| i.to_string())
    }

    /// The world, briefly, in terms that are the same every run.
    async fn state(&self) -> String {
        let records = self
            .iglud
            .app
            .db
            .call(|tx| {
                Ok(db::live_workspaces(tx)?
                    .into_iter()
                    .map(|ws| (ws.id, ws.desired, ws.condition.is_some()))
                    .collect::<Vec<_>>())
            })
            .await
            .expect("reading workspaces");
        let held = self.runtime.instances_held();
        let mut out = String::new();
        for (i, w) in self.workspaces.iter().enumerate() {
            let record = records.iter().find(|r| r.0 == w.id);
            let instance = held.iter().find(|(n, _)| *n == w.id.instance_name());
            let _ = write!(
                out,
                "w{i}:{}/{}{} ",
                record.map_or("gone".into(), |r| format!("{:?}", r.1)),
                instance.map_or("-".into(), |(_, i)| format!(
                    "{}{}{}{}",
                    power(i.power),
                    if i.guest.secrets.is_some() { "+s" } else { "" },
                    if i.guest.columns_opened { "+c" } else { "" },
                    i.guest.sessions.len()
                )),
                if record.is_some_and(|r| r.2) { "!" } else { "" },
            );
        }
        let _ = write!(out, "pending {}", self.runtime.pending().len());
        out
    }

    /// Stops the faults and lets everything finish, then checks that every
    /// workspace is as its owner asked.
    async fn converge(&mut self) {
        self.trace.push("faults stop".into());
        for _ in 0..60 {
            if !self.settle().await {
                return;
            }
            self.harvest();
            let pending = self.pending();
            if let Some(next) = pending.first() {
                self.runtime.resolve(next.seq, Fault::None);
                continue;
            }
            tokio::time::advance(QUIET).await;
            crate::reconcile::pass(&self.iglud.app, &self.iglud.app.reconciler).await;
            if !self.settle().await {
                return;
            }
            self.harvest();
            if self.pending().is_empty()
                && self.asked.is_empty()
                && self.converged().await.is_empty()
            {
                return;
            }
        }
        for problem in self.converged().await {
            self.find(format!("never converged: {problem}"));
        }
    }

    /// What isn't yet as its owner asked.
    async fn converged(&self) -> Vec<String> {
        let people: Vec<PrincipalId> = self.people.iter().map(|p| p.principal).collect();
        let (records, generations, images) = self
            .iglud
            .app
            .db
            .call(move |tx| {
                let records = db::live_workspaces(tx)?;
                let mut generations = HashMap::new();
                for person in people {
                    if let Some(record) = db::principal(tx, person)? {
                        generations.insert(person, record.secrets_generation);
                    }
                }
                Ok((records, generations, db::workspace_images(tx)?))
            })
            .await
            .expect("reading workspaces");
        let held = self.runtime.instances_held();
        let mut problems = Vec::new();
        for (name, _) in &held {
            if !records.iter().any(|r| r.id.instance_name() == *name) {
                let w = self.index(name.workspace());
                problems.push(format!("w{w}'s instance outlived its workspace"));
            }
        }
        for record in &records {
            let w = self.index(record.id);
            let instance = held
                .iter()
                .find(|(n, _)| *n == record.id.instance_name())
                .map(|(_, i)| i);
            let wanted = match record.desired {
                DesiredState::Running => "running",
                DesiredState::Frozen => "frozen",
                DesiredState::Stopped => "stopped",
                DesiredState::Deleted => "absent",
            };
            let shown = instance.map_or("absent", |i| power(i.power));
            // A failed instance waits for its owner to stop or delete it.
            let blocked = shown == "failed"
                && matches!(record.desired, DesiredState::Running | DesiredState::Frozen)
                && matches!(record.condition, Some(Condition::RuntimeFailed));
            if blocked {
                continue;
            }
            if wanted != shown {
                problems.push(format!("w{w} is {shown}, not {wanted}"));
                continue;
            }
            if let Some(condition) = &record.condition {
                problems.push(format!("w{w} keeps a condition: {}", described(condition)));
            }
            let Some(instance) = instance else { continue };
            if record.desired == DesiredState::Running {
                let generation = generations.get(&record.owner).copied();
                if instance.guest.secrets != generation {
                    problems.push(format!(
                        "w{w} holds secrets {:?}, not {generation:?}",
                        instance.guest.secrets
                    ));
                }
                if !instance.guest.columns_opened {
                    problems.push(format!("w{w}'s columns never opened"));
                }
                if instance.provisioning != iglu_domain::lifecycle::Provisioning::Complete {
                    problems.push(format!("w{w} was never provisioned"));
                }
            }
            if images
                .get(&record.id)
                .is_some_and(|image| *image != instance.image)
            {
                problems.push(format!("w{w} runs an image other than its revision's"));
            }
            let observed = record.observed.map_or("absent", kind);
            if observed != shown {
                problems.push(format!(
                    "iglud records w{w} as {observed}, but it's {shown}"
                ));
            }
        }
        problems
    }
}

/// A condition, without what differs between runs: addresses, IDs and
/// times.
fn described(condition: &Condition) -> String {
    match condition {
        Condition::Error { code, message, .. } => {
            let message = message.split(" (").next().unwrap_or(message);
            format!("error {code:?}: {message}")
        }
        other @ (Condition::Capacity { .. }
        | Condition::RuntimeFailed
        | Condition::HostOffline { .. }) => format!("{other:?}"),
    }
}

const LIFECYCLE: [&str; 6] = ["start", "freeze", "thaw", "stop", "delete", "upgrade"];

fn power(power: Power) -> &'static str {
    match power {
        Power::Stopped => "stopped",
        Power::Running(_) => "running",
        Power::Frozen(_) => "frozen",
        Power::Failed => "failed",
    }
}

fn kind(instance: Instance) -> &'static str {
    match instance {
        Instance::Absent => "absent",
        Instance::Present(Present { runtime, .. }) => match runtime {
            Status::Stopped => "stopped",
            Status::Running(_) => "running",
            Status::Frozen => "frozen",
            Status::Transitioning => "transitioning",
            Status::Failed => "failed",
        },
    }
}

/// Runs a simulation in a runtime of its own, whose clock moves only when
/// the driver moves it.
fn simulated<F: std::future::Future>(simulation: F) -> F::Output {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .expect("a runtime");
    let output = runtime.block_on(async {
        // A blocking task under way keeps a paused clock from moving on its
        // own.
        let (keep, kept) = std::sync::mpsc::channel::<()>();
        let still = tokio::task::spawn_blocking(move || {
            let _ = kept.recv();
        });
        let output = simulation.await;
        drop(keep);
        let _ = still.await;
        output
    });
    runtime.shutdown_background();
    output
}

/// Runs one schedule of `steps` events from `seed`.
pub fn schedule(seed: u64, steps: usize, dir: &Path) -> Report {
    let dir = dir.join(format!("{seed}"));
    let report = simulated(async {
        let mut sim = Sim::new(seed, dir.clone()).await;
        for _ in 0..steps {
            sim.step().await;
            if !sim.findings.is_empty() {
                break;
            }
        }
        if sim.findings.is_empty() {
            sim.converge().await;
        }
        let digest = hex::encode(Sha256::digest(sim.trace.join("\n").as_bytes()));
        Report {
            refused_while_changing: sim.runtime.refused_while_changing(),
            findings: sim.findings,
            digest,
            trace: sim.trace,
        }
    });
    if report.findings.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    report
}

/// Lets everything pending finish and the reconciler act, until there's
/// nothing left to do.
async fn drive(sim: &mut Sim) {
    for _ in 0..40 {
        sim.settle().await;
        sim.harvest();
        if let Some(next) = sim.pending().first() {
            sim.runtime.resolve(next.seq, Fault::None);
            continue;
        }
        tokio::time::advance(QUIET).await;
        crate::reconcile::pass(&sim.iglud.app, &sim.iglud.app.reconciler).await;
        sim.settle().await;
        if sim.pending().is_empty() {
            return;
        }
    }
}

/// The smallest schedule that shows a settled workspace keeping an error:
/// a step that takes effect but whose answer is lost leaves the condition
/// it recorded, and with nothing left to do, nothing clears it.
async fn stale_error(dir: PathBuf) -> Vec<String> {
    let mut sim = Sim::new(0, dir).await;
    sim.create();
    drive(&mut sim).await;
    sim.ask(
        sim.workspaces[0].owner,
        Some(0),
        "desire",
        Method::PUT,
        format!("/v1/workspaces/{}/desired-state", sim.workspaces[0].id),
        Some(json!({"state": "stopped"})),
        Then::Nothing,
    );
    sim.settle().await;
    crate::reconcile::pass(&sim.iglud.app, &sim.iglud.app.reconciler).await;
    sim.settle().await;
    let stop = sim
        .pending()
        .into_iter()
        .find(|p| p.what == "stop")
        .expect("a stop is pending");
    sim.runtime.resolve(stop.seq, Fault::After);
    drive(&mut sim).await;
    sim.converged().await
}

/// The smallest schedule that shows a guest left with older secrets than
/// iglud and hostd believe it has: a delivery under way when iglud restarts
/// carries on, the new iglud delivers newer secrets, and the older delivery
/// lands last.
async fn older_secrets(dir: PathBuf) -> Vec<String> {
    let mut sim = Sim::new(0, dir).await;
    sim.create();
    drive(&mut sim).await;
    let owner = sim.workspaces[0].owner;
    let deliver = |sim: &Sim, orphaned: bool| {
        sim.pending()
            .into_iter()
            .find(|p| p.what == "install-secrets" && p.orphaned == orphaned)
    };
    sim.change_secret(owner);
    sim.settle().await;
    crate::reconcile::pass(&sim.iglud.app, &sim.iglud.app.reconciler).await;
    sim.settle().await;
    assert!(deliver(&sim, false).is_some(), "a delivery is under way");
    sim.change_secret(owner);
    sim.settle().await;
    sim.restart_iglud().await;
    sim.settle().await;
    crate::reconcile::pass(&sim.iglud.app, &sim.iglud.app.reconciler).await;
    sim.settle().await;
    let newer = deliver(&sim, false).expect("the new iglud delivers");
    sim.runtime.resolve(newer.seq, Fault::None);
    sim.settle().await;
    let older = deliver(&sim, true).expect("the older delivery carries on");
    sim.runtime.resolve(older.seq, Fault::None);
    drive(&mut sim).await;
    sim.converged().await
}

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn scratch() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/sim");
    std::fs::create_dir_all(&dir).expect("made");
    dir
}

/// Findings already reported, which a schedule may come to without the
/// test failing, each with the test that reproduces it. Take one out when
/// it's fixed.
const KNOWN: &[(&str, &str)] = &[
    (
        "keeps a condition: error",
        "a_settled_workspace_keeps_no_error",
    ),
    ("secrets went back", "secrets_never_go_back"),
    ("holds secrets", "secrets_never_go_back"),
];

#[test]
fn schedules_keep_the_invariants_and_converge() {
    let first = setting("IGLU_SIM_SEED", 1);
    let schedules = setting("IGLU_SIM_SCHEDULES", 8);
    let steps = usize::try_from(setting("IGLU_SIM_STEPS", 120)).expect("a step count");
    let dir = scratch();
    let started = WallClock::now();
    let mut failed = Vec::new();
    let mut known = 0;
    let mut refused = 0;
    for seed in first..first + schedules {
        let report = schedule(seed, steps, &dir);
        refused += report.refused_while_changing;
        let (old, new): (Vec<String>, Vec<String>) = report
            .findings
            .into_iter()
            .partition(|f| KNOWN.iter().any(|(pattern, _)| f.contains(pattern)));
        known += usize::from(!old.is_empty());
        if !new.is_empty() {
            eprintln!("seed {seed}:\n{}", report.trace.join("\n"));
            failed.push((seed, new));
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    eprintln!(
        "{schedules} schedules of {steps} events in {elapsed:.1}s: {:.1} schedules/s, \
         {known} with known findings, {refused} steps refused while changing",
        f64::from(u32::try_from(schedules).unwrap_or(u32::MAX)) / elapsed
    );
    assert!(failed.is_empty(), "findings: {failed:#?}");
}

#[test]
fn seeds_replay() {
    let dir = scratch().join("replay");
    let first = setting("IGLU_SIM_SEED", 7);
    let steps = usize::try_from(setting("IGLU_SIM_STEPS", 80)).expect("a step count");
    for seed in first..first + setting("IGLU_SIM_REPLAYS", 2) {
        let once = schedule(seed, steps, &dir);
        let again = schedule(seed, steps, &dir);
        if let Some(at) = (0..once.trace.len().max(again.trace.len()))
            .find(|&i| once.trace.get(i) != again.trace.get(i))
        {
            let from = at.saturating_sub(12);
            panic!(
                "seed {seed} played two ways from event line {at}:\n{}\n--- once: {:?}\n--- again: {:?}",
                once.trace[from..at].join("\n"),
                once.trace.get(at),
                again.trace.get(at),
            );
        }
        assert_eq!(once.digest, again.digest);
    }
}

#[test]
#[ignore = "reproduces a finding: a settled workspace keeps a stale error condition"]
fn a_settled_workspace_keeps_no_error() {
    let mut problems = simulated(stale_error(scratch().join("stale-error")));
    problems.retain(|p| p.contains("condition"));
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
#[ignore = "reproduces a finding: a guest can be left with older secrets than iglud believes"]
fn secrets_never_go_back() {
    let mut problems = simulated(older_secrets(scratch().join("older-secrets")));
    problems.retain(|p| p.contains("secrets"));
    assert_eq!(problems, Vec::<String>::new());
}
