use crate::{
    context::{Client, TracedChannel},
    ha_cluster_agent::{ha_rpc_client::HaRpcClient, HaNodeInfo},
    operations::ha_node::traits::{ClusterAgentOperations, NodeInfo},
};
use common_lib::transport_api::{ReplyError, TimeoutOptions};
use std::ops::Deref;
use tonic::transport::Uri;

/// Cluster-Agent RPC client
pub struct ClusterAgentClient {
    inner: Client<HaRpcClient<TracedChannel>>,
}

impl ClusterAgentClient {
    /// creates a new base tonic endpoint with the timeout options and the address
    pub async fn new<O: Into<Option<TimeoutOptions>>>(addr: Uri, opts: O) -> Self {
        let client = Client::new(addr, opts, HaRpcClient::new).await;
        Self { inner: client }
    }
}

impl Deref for ClusterAgentClient {
    type Target = Client<HaRpcClient<TracedChannel>>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[tonic::async_trait]
impl ClusterAgentOperations for ClusterAgentClient {
    #[tracing::instrument(
        name = "ClusterAgentClient::register_node_agent",
        level = "debug",
        skip(self),
        err
    )]
    async fn register(&self, request: &dyn NodeInfo) -> Result<(), ReplyError> {
        let _response = self
            .client()
            .register_node_agent(HaNodeInfo {
                nodename: request.node(),
                endpoint: request.endpoint(),
            })
            .await?;
        tracing::trace!("node agent successfully registered");
        Ok(())
    }
}
