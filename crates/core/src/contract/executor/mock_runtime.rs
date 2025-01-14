use super::*;

pub(crate) struct MockRuntime {
    pub contract_store: ContractStore,
}

impl Executor<MockRuntime> {
    pub async fn new_mock(
        identifier: &str,
        event_loop_channel: ExecutorToEventLoopChannel<ExecutorHalve>,
    ) -> Result<Self, DynError> {
        let data_dir = std::env::temp_dir().join(format!("freenet-executor-{identifier}"));

        let contracts_data_dir = data_dir.join("contracts");
        std::fs::create_dir_all(&contracts_data_dir).expect("directory created");
        let contract_store = ContractStore::new(contracts_data_dir, u16::MAX as i64)?;

        let db_path = data_dir.join("db");
        std::fs::create_dir_all(&db_path).expect("directory created");
        let log_file = data_dir.join("_EVENT_LOG_LOCAL");
        crate::config::Config::set_event_log(log_file);
        let state_store =
            StateStore::new(Storage::new(Some(&db_path)).await?, u16::MAX as u32).unwrap();

        let executor = Executor::new(
            state_store,
            || Ok(()),
            OperationMode::Local,
            MockRuntime { contract_store },
            Some(event_loop_channel),
        )
        .await?;
        Ok(executor)
    }

    pub async fn handle_request<'a>(
        &mut self,
        _id: ClientId,
        _req: ClientRequest<'a>,
        _updates: Option<mpsc::UnboundedSender<Result<HostResponse, WsClientError>>>,
    ) -> Response {
        unreachable!()
    }
}

#[async_trait::async_trait]
impl ContractExecutor for Executor<MockRuntime> {
    async fn fetch_contract(
        &mut self,
        key: ContractKey,
        _fetch_contract: bool,
    ) -> Result<(WrappedState, Option<ContractContainer>), ExecutorError> {
        let Some(parameters) = self
            .state_store
            .get_params(&key)
            .await
            .map_err(ExecutorError::other)?
        else {
            return Err(ExecutorError::other(format!(
                "missing parameters for contract {key}"
            )));
        };
        let Ok(state) = self.state_store.get(&key).await else {
            return Err(ExecutorError::other(format!(
                "missing state for contract {key}"
            )));
        };
        let contract = self
            .runtime
            .contract_store
            .fetch_contract(&key, &parameters);
        Ok((state, contract))
    }

    async fn store_contract(&mut self, contract: ContractContainer) -> Result<(), ExecutorError> {
        self.runtime
            .contract_store
            .store_contract(contract)
            .map_err(ExecutorError::other)?;
        Ok(())
    }

    async fn upsert_contract_state(
        &mut self,
        key: ContractKey,
        state: Either<WrappedState, StateDelta<'static>>,
        related_contracts: RelatedContracts<'static>,
        code: Option<ContractContainer>,
    ) -> Result<WrappedState, ExecutorError> {
        // todo: instead allow to perform mutations per contract based on incoming value so we can track
        // state values over the network
        match (state, code) {
            (Either::Left(incoming_state), Some(contract)) => {
                self.state_store
                    .store(key, incoming_state.clone(), contract.params().into_owned())
                    .await
                    .map_err(ExecutorError::other)?;

                let request = PutContract {
                    contract,
                    state: incoming_state.clone(),
                    related_contracts,
                };
                let _op: Result<operations::put::PutResult, _> = self.op_request(request).await;

                return Ok(incoming_state);
            }
            _ => unreachable!(),
        }
    }

    async fn subscribe_to_contract(
        &mut self,
        key: ContractKey,
    ) -> Result<PeerKeyLocation, ExecutorError> {
        let request = SubscribeContract { key };
        let result: operations::subscribe::SubscribeResult = self.op_request(request).await?;
        Ok(result.subscribed_to)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::wasm_runtime::{ContractStore, StateStore};

    #[tokio::test(flavor = "multi_thread")]
    async fn local_node_handle() -> Result<(), Box<dyn std::error::Error>> {
        const MAX_SIZE: i64 = 10 * 1024 * 1024;
        const MAX_MEM_CACHE: u32 = 10_000_000;
        let tmp_dir = tempfile::tempdir()?;
        let contract_store = ContractStore::new(tmp_dir.path().join("executor-test"), MAX_SIZE)?;
        let state_store = StateStore::new(Storage::new(None).await?, MAX_MEM_CACHE).unwrap();
        let mut counter = 0;
        Executor::new(
            state_store,
            || {
                counter += 1;
                Ok(())
            },
            OperationMode::Local,
            MockRuntime { contract_store },
            None,
        )
        .await
        .expect("local node with handle");

        assert_eq!(counter, 1);
        Ok(())
    }
}
