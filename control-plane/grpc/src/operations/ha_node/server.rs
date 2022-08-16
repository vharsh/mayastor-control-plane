use tonic::{Response, Status};

use crate::{
    ha_cluster_agent::{
        ha_rpc_server::{HaRpc, HaRpcServer},
        HaNodeInfo, ReportFailedNvmePathsRequest,
    },
    operations::ha_node::traits::ClusterAgentOperations,
};
use std::sync::Arc;

/// RPC cluster-agent server
pub struct ClusterAgentServer {
    service: Arc<dyn ClusterAgentOperations>,
}

impl ClusterAgentServer {
    /// returns a new cluster-agent server with the service implementing cluster-agent operations
    pub fn new(svc: Arc<dyn ClusterAgentOperations>) -> Self {
        ClusterAgentServer { service: svc }
    }

    /// converts the cluster-agent server to corresponding grpc server type
    pub fn into_grpc_server(self) -> HaRpcServer<Self> {
        HaRpcServer::new(self)
    }
}

#[tonic::async_trait]
impl HaRpc for ClusterAgentServer {
    async fn register_node_agent(
        &self,
        request: tonic::Request<HaNodeInfo>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        let nodeinfo = request.into_inner();
        match self.service.register(&nodeinfo).await {
            Ok(_) => Ok(Response::new(())),
            Err(err) => Err(Status::internal(format!(
                "Failed to register node-agent: {:?}",
                err
            ))),
        }
    }
    async fn report_failed_nvme_paths(
        &self,
        _request: tonic::Request<ReportFailedNvmePathsRequest>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        Err(Status::unimplemented(
            "NVMe path reporting is not yet implemented",
        ))
    }
}