use composer::*;
use deployer_lib::{
    infra::{Components, Error, Mayastor},
    *,
};
use opentelemetry::{
    global,
    sdk::{propagation::TraceContextPropagator, trace::Tracer},
};

pub use common_lib::{
    mbus_api,
    mbus_api::{Message, TimeoutOptions},
    types::v0::{
        message_bus::{self, PoolDeviceUri},
        openapi::{apis::Uuid, models},
    },
};
pub use rest_client::ActixRestClient;

pub mod v0 {
    pub use common_lib::{
        mbus_api,
        types::v0::{
            message_bus::{
                AddNexusChild, BlockDevice, Child, ChildUri, CreateNexus, CreatePool,
                CreateReplica, CreateVolume, DestroyNexus, DestroyPool, DestroyReplica,
                DestroyVolume, Filter, GetBlockDevices, JsonGrpcRequest, Nexus, NexusId, Node,
                NodeId, Pool, PoolDeviceUri, PoolId, Protocol, RemoveNexusChild, Replica,
                ReplicaId, ReplicaShareProtocol, ShareNexus, ShareReplica, Specs, Topology,
                UnshareNexus, UnshareReplica, VolumeHealPolicy, VolumeId, Watch, WatchCallback,
                WatchResourceId,
            },
            openapi::{apis, models},
        },
    };
    pub use models::rest_json_error::Kind as RestJsonErrorKind;
}

use std::{collections::HashMap, rc::Rc, time::Duration};

#[actix_rt::test]
#[ignore]
async fn smoke_test() {
    // make sure the cluster can bootstrap properly
    let _cluster = ClusterBuilder::builder()
        .build()
        .await
        .expect("Should bootstrap the cluster!");
}

/// Default options to create a cluster
pub fn default_options() -> StartOptions {
    StartOptions::default()
        .with_agents(default_agents().split(',').collect())
        .with_jaeger(true)
        .with_mayastors(1)
        .with_show_info(true)
        .with_cluster_name("rest_cluster")
        .with_build_all(true)
}

/// Cluster with the composer, the rest client and the jaeger pipeline#
#[allow(unused)]
pub struct Cluster {
    composer: ComposeTest,
    rest_client: ActixRestClient,
    jaeger: Tracer,
    builder: ClusterBuilder,
}

impl Cluster {
    /// compose utility
    pub fn composer(&self) -> &ComposeTest {
        &self.composer
    }

    /// node id for `index`
    pub fn node(&self, index: u32) -> message_bus::NodeId {
        Mayastor::name(index, &self.builder.opts).into()
    }

    /// node ip for `index`
    pub fn node_ip(&self, index: u32) -> String {
        let name = self.node(index);
        self.composer.container_ip(name.as_str())
    }

    /// pool id for `pool` index on `node` index
    pub fn pool(&self, node: u32, pool: u32) -> message_bus::PoolId {
        format!("{}-pool-{}", self.node(node), pool + 1).into()
    }

    /// replica id with index for `pool` index and `replica` index
    pub fn replica(node: u32, pool: usize, replica: u32) -> message_bus::ReplicaId {
        let mut uuid = message_bus::ReplicaId::default().to_string();
        let _ = uuid.drain(24 .. uuid.len());
        format!(
            "{}{:02x}{:02x}{:08x}",
            uuid, node as u8, pool as u8, replica
        )
        .into()
    }

    /// openapi rest client v0
    pub fn rest_v00(&self) -> common_lib::types::v0::openapi::ApiClient {
        self.rest_client.v00()
    }

    /// New cluster
    async fn new(
        trace_rest: bool,
        timeout_rest: std::time::Duration,
        bus_timeout: TimeoutOptions,
        bearer_token: Option<String>,
        components: Components,
        composer: ComposeTest,
        jaeger: Tracer,
    ) -> Result<Cluster, Error> {
        let rest_client = ActixRestClient::new_timeout(
            "http://localhost:8081",
            trace_rest,
            bearer_token,
            timeout_rest,
        )
        .unwrap();

        components
            .start_wait(&composer, std::time::Duration::from_secs(30))
            .await?;

        let cluster = Cluster {
            composer,
            rest_client,
            jaeger,
            builder: ClusterBuilder::builder(),
        };

        // the deployer uses a "fake" message bus so now it's time to
        // connect to the "real" message bus
        cluster.connect_to_bus_timeout("nats", bus_timeout).await;

        Ok(cluster)
    }

    /// connect to message bus helper for the cargo test code
    #[allow(dead_code)]
    async fn connect_to_bus(&self, name: &str) {
        let timeout = TimeoutOptions::new()
            .with_timeout(Duration::from_millis(500))
            .with_timeout_backoff(Duration::from_millis(500))
            .with_max_retries(10);
        self.connect_to_bus_timeout(name, timeout).await;
    }

    /// connect to message bus helper for the cargo test code with bus timeouts
    async fn connect_to_bus_timeout(&self, name: &str, bus_timeout: TimeoutOptions) {
        actix_rt::time::timeout(std::time::Duration::from_secs(2), async {
            mbus_api::message_bus_init_options(self.composer.container_ip(name), bus_timeout).await
        })
        .await
        .unwrap();
    }
}

fn option_str<F: ToString>(input: Option<F>) -> String {
    match input {
        Some(input) => input.to_string(),
        None => "?".into(),
    }
}

/// Run future and compare result with what's expected
/// Expected result should be in the form Result<TestValue,TestValue>
/// where TestValue is a useful value which will be added to the returned error
/// string Eg, testing the replica share protocol:
/// test_result(Ok(Nvmf), async move { ... })
/// test_result(Err(NBD), async move { ... })
pub async fn test_result<F, O, E, T>(
    expected: &Result<O, E>,
    future: F,
) -> Result<(), anyhow::Error>
where
    F: std::future::Future<Output = Result<T, common_lib::mbus_api::Error>>,
    E: std::fmt::Debug,
    O: std::fmt::Debug,
{
    match future.await {
        Ok(_) if expected.is_ok() => Ok(()),
        Err(error) if expected.is_err() => match error {
            common_lib::mbus_api::Error::ReplyWithError { .. } => Ok(()),
            _ => {
                // not the error we were waiting for
                Err(anyhow::anyhow!("Invalid response: {:?}", error))
            }
        },
        Err(error) => Err(anyhow::anyhow!(
            "Expected '{:#?}' but failed with '{:?}'!",
            expected,
            error
        )),
        Ok(_) => Err(anyhow::anyhow!("Expected '{:#?}' but succeeded!", expected)),
    }
}

#[macro_export]
macro_rules! result_either {
    ($test:expr) => {
        match $test {
            Ok(v) => v,
            Err(v) => v,
        }
    };
}

#[derive(Clone)]
enum PoolDisk {
    Malloc(u64),
    Uri(String),
    Tmp(TmpDiskFile),
}

/// Temporary "disk" file, which gets deleted on drop
#[derive(Clone)]
pub struct TmpDiskFile {
    inner: Rc<TmpDiskFileInner>,
}

struct TmpDiskFileInner {
    path: String,
    uri: String,
}

impl TmpDiskFile {
    /// Creates a new file on `path` with `size`.
    /// The file is deleted on drop.
    pub fn new(name: &str, size: u64) -> Self {
        Self {
            inner: Rc::new(TmpDiskFileInner::new(name, size)),
        }
    }
    /// Disk URI to be used by mayastor
    pub fn uri(&self) -> &str {
        self.inner.uri()
    }
}
impl TmpDiskFileInner {
    fn new(name: &str, size: u64) -> Self {
        let path = format!("/tmp/mayastor-{}", name);
        let file = std::fs::File::create(&path).expect("to create the tmp file");
        file.set_len(size).expect("to truncate the tmp file");
        Self {
            // mayastor is setup with a bind mount from /tmp to /host/tmp
            uri: format!(
                "aio:///host{}?blk_size=512&uuid={}",
                path,
                message_bus::PoolId::new().to_string()
            ),
            path,
        }
    }
    fn uri(&self) -> &str {
        &self.uri
    }
}

impl Drop for TmpDiskFileInner {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).expect("to unlink the tmp file");
    }
}

/// Builder for the Cluster
pub struct ClusterBuilder {
    opts: StartOptions,
    pools: HashMap<u32, Vec<PoolDisk>>,
    replicas: Replica,
    trace: bool,
    bearer_token: Option<String>,
    rest_timeout: std::time::Duration,
    bus_timeout: TimeoutOptions,
}

#[derive(Default)]
struct Replica {
    count: u32,
    size: u64,
    share: message_bus::Protocol,
}

/// default timeout options for every bus request
fn bus_timeout_opts() -> TimeoutOptions {
    TimeoutOptions::default()
        .with_timeout(Duration::from_secs(5))
        .with_timeout_backoff(Duration::from_millis(500))
        .with_max_retries(2)
}

impl ClusterBuilder {
    /// Cluster Builder with default options
    pub fn builder() -> Self {
        ClusterBuilder {
            opts: default_options(),
            pools: Default::default(),
            replicas: Default::default(),
            trace: true,
            bearer_token: None,
            rest_timeout: std::time::Duration::from_secs(5),
            bus_timeout: bus_timeout_opts(),
        }
    }
    /// Update the start options
    pub fn with_options<F>(mut self, set: F) -> Self
    where
        F: Fn(StartOptions) -> StartOptions,
    {
        self.opts = set(self.opts);
        self
    }
    /// Enable/Disable jaeger tracing
    pub fn with_tracing(mut self, enabled: bool) -> Self {
        self.trace = enabled;
        self
    }
    /// Rest request timeout
    pub fn with_rest_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.rest_timeout = timeout;
        self
    }
    /// Add `count` malloc pools (100MiB size) to each node
    pub fn with_pools(mut self, count: u32) -> Self {
        for _ in 0 .. count {
            for node in 0 .. self.opts.mayastors {
                if let Some(pools) = self.pools.get_mut(&node) {
                    pools.push(PoolDisk::Malloc(100 * 1024 * 1024));
                } else {
                    self.pools
                        .insert(node, vec![PoolDisk::Malloc(100 * 1024 * 1024)]);
                }
            }
        }
        self
    }
    /// Add pool URI with `disk` to the node `index`
    pub fn with_pool(mut self, index: u32, disk: &str) -> Self {
        if let Some(pools) = self.pools.get_mut(&index) {
            pools.push(PoolDisk::Uri(disk.to_string()));
        } else {
            self.pools
                .insert(index, vec![PoolDisk::Uri(disk.to_string())]);
        }
        self
    }
    /// Add a tmpfs img pool with `disk` to each mayastor node with the specified `size`
    pub fn with_tmpfs_pool(mut self, size: u64) -> Self {
        for node in 0 .. self.opts.mayastors {
            let disk = TmpDiskFile::new(&Uuid::new_v4().to_string(), size);
            if let Some(pools) = self.pools.get_mut(&node) {
                pools.push(PoolDisk::Tmp(disk));
            } else {
                self.pools.insert(node, vec![PoolDisk::Tmp(disk)]);
            }
        }
        self
    }
    /// Specify `count` replicas to add to each node per pool
    pub fn with_replicas(mut self, count: u32, size: u64, share: message_bus::Protocol) -> Self {
        self.replicas = Replica { count, size, share };
        self
    }
    /// Specify `count` mayastors for the cluster
    pub fn with_mayastors(mut self, count: u32) -> Self {
        self.opts = self.opts.with_mayastors(count);
        self
    }
    /// Specify which agents to use
    pub fn with_agents(mut self, agents: Vec<&str>) -> Self {
        self.opts = self.opts.with_agents(agents);
        self
    }
    /// Specify the node deadline for the node agent
    /// eg: 2s
    pub fn with_node_deadline(mut self, deadline: &str) -> Self {
        self.opts = self.opts.with_node_deadline(deadline);
        self
    }
    /// The period at which the registry updates its cache of all
    /// resources from all nodes
    pub fn with_cache_period(mut self, period: &str) -> Self {
        self.opts = self.opts.with_cache_period(period);
        self
    }

    /// With reconcile periods:
    /// `busy` for when there's work that needs to be retried on the next poll
    /// `idle` when there's no work pending
    pub fn with_reconcile_period(mut self, busy: Duration, idle: Duration) -> Self {
        self.opts = self.opts.with_reconcile_period(busy, idle);
        self
    }
    /// With store operation timeout
    pub fn with_store_timeout(mut self, timeout: Duration) -> Self {
        self.opts = self.opts.with_store_timeout(timeout);
        self
    }
    /// Specify the node connect and request timeouts
    pub fn with_node_timeouts(mut self, connect: Duration, request: Duration) -> Self {
        self.opts = self.opts.with_node_timeouts(connect, request);
        self
    }
    /// Specify the message bus timeout options
    pub fn with_bus_timeouts(mut self, timeout: TimeoutOptions) -> Self {
        self.bus_timeout = timeout;
        self
    }
    /// Specify whether rest is enabled or not
    pub fn with_rest(mut self, enabled: bool) -> Self {
        self.opts = self.opts.with_rest(enabled, None);
        self
    }
    /// Specify whether rest is enabled or not and wether to use authentication or not
    pub fn with_rest_auth(mut self, enabled: bool, jwk: Option<String>) -> Self {
        self.opts = self.opts.with_rest(enabled, jwk);
        self
    }
    /// Specify whether the components should be cargo built or not
    pub fn with_build(mut self, enabled: bool) -> Self {
        self.opts = self.opts.with_build(enabled);
        self
    }
    /// Specify whether the workspace binaries should be cargo built or not
    pub fn with_build_all(mut self, enabled: bool) -> Self {
        self.opts = self.opts.with_build_all(enabled);
        self
    }
    /// Build into the resulting Cluster using a composer closure, eg:
    /// .compose_build(|c| c.with_logs(false))
    pub async fn compose_build<F>(self, set: F) -> Result<Cluster, Error>
    where
        F: Fn(Builder) -> Builder,
    {
        let (components, composer) = self.build_prepare()?;
        let composer = set(composer);
        let mut cluster = self.new_cluster(components, composer).await?;
        cluster.builder = self;
        Ok(cluster)
    }
    /// Build into the resulting Cluster
    pub async fn build(self) -> Result<Cluster, Error> {
        let (components, composer) = self.build_prepare()?;
        let mut cluster = self.new_cluster(components, composer).await?;
        cluster.builder = self;
        Ok(cluster)
    }
    fn build_prepare(&self) -> Result<(Components, Builder), Error> {
        let components = Components::new(self.opts.clone());
        let composer = Builder::new()
            .name(&self.opts.cluster_label.name())
            .configure(components.clone())?
            .with_base_image(self.opts.base_image.clone())
            .autorun(false)
            .with_default_tracing()
            .with_clean(true)
            // test script will clean up containers if ran on CI/CD
            .with_clean_on_panic(false)
            .with_logs(true);
        Ok((components, composer))
    }
    async fn new_cluster(
        &self,
        components: Components,
        compose_builder: Builder,
    ) -> Result<Cluster, Error> {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let jaeger = opentelemetry_jaeger::new_pipeline()
            .with_service_name("tests-client")
            .install_simple()
            .unwrap();

        let composer = compose_builder.build().await?;

        let cluster = Cluster::new(
            self.trace,
            self.rest_timeout,
            self.bus_timeout.clone(),
            self.bearer_token.clone(),
            components,
            composer,
            jaeger,
        )
        .await?;

        if self.opts.show_info {
            for container in cluster.composer.list_cluster_containers().await? {
                let networks = container.network_settings.unwrap().networks.unwrap();
                let ip = networks
                    .get(&self.opts.cluster_label.name())
                    .unwrap()
                    .ip_address
                    .clone();
                tracing::debug!(
                    "{:?} [{}] {}",
                    container.names.clone().unwrap_or_default(),
                    ip.clone().unwrap_or_default(),
                    option_str(container.command.clone())
                );
            }
        }

        for pool in &self.pools() {
            message_bus::CreatePool {
                node: pool.node.clone().into(),
                id: pool.id(),
                disks: vec![pool.disk()],
            }
            .request()
            .await
            .unwrap();

            for replica in &pool.replicas {
                replica.request().await.unwrap();
            }
        }

        Ok(cluster)
    }
    fn pools(&self) -> Vec<Pool> {
        let mut pools = vec![];

        for (node, i_pools) in &self.pools {
            for (pool_index, pool) in i_pools.iter().enumerate() {
                let mut pool = Pool {
                    node: Mayastor::name(*node, &self.opts),
                    disk: pool.clone(),
                    index: (pool_index + 1) as u32,
                    replicas: vec![],
                };
                for replica_index in 0 .. self.replicas.count {
                    pool.replicas.push(message_bus::CreateReplica {
                        node: pool.node.clone().into(),
                        uuid: Cluster::replica(*node, pool_index, replica_index),
                        pool: pool.id(),
                        size: self.replicas.size,
                        thin: false,
                        share: self.replicas.share,
                        managed: false,
                        owners: Default::default(),
                    });
                }
                pools.push(pool);
            }
        }
        pools
    }
}

struct Pool {
    node: String,
    disk: PoolDisk,
    index: u32,
    replicas: Vec<message_bus::CreateReplica>,
}

impl Pool {
    fn id(&self) -> message_bus::PoolId {
        format!("{}-pool-{}", self.node, self.index).into()
    }
    fn disk(&self) -> PoolDeviceUri {
        match &self.disk {
            PoolDisk::Malloc(size) => {
                let size = size / (1024 * 1024);
                format!(
                    "malloc:///disk{}?size_mb={}&uuid={}",
                    self.index,
                    size,
                    message_bus::PoolId::new()
                )
                .into()
            }
            PoolDisk::Uri(uri) => uri.into(),
            PoolDisk::Tmp(disk) => disk.uri().into(),
        }
    }
}
