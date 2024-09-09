use anyhow::Result;
use automerge::{
    sync::{self, SyncDoc},
    transaction::Transactable,
    Automerge,
};
use iroh::{
    net::endpoint::{RecvStream, SendStream},
    node::ProtocolHandler,
};
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Protocol {
    SyncMessage(Vec<u8>),
    UserInput { key: String, value: String },
    Done,
}

#[derive(Debug)]
pub struct IrohAutomergeProtocol {
    inner: Mutex<Automerge>,
    sync_finished: mpsc::Sender<Automerge>,
    user_input: broadcast::Sender<(String, String)>,
    doc_update: watch::Sender<Automerge>,
}

impl IrohAutomergeProtocol {
    pub const ALPN: &'static [u8] = b"iroh/automerge/1";
    pub fn new(
        doc: Automerge,
        sync_finished: mpsc::Sender<Automerge>,
        doc_update: watch::Sender<Automerge>,
    ) -> (Arc<Self>, broadcast::Receiver<(String, String)>) {
        let (user_input_tx, user_input_rx) = broadcast::channel(100);
        (
            Arc::new(Self {
                inner: Mutex::new(doc),
                sync_finished,
                user_input: user_input_tx,
                doc_update,
            }),
            user_input_rx,
        )
    }

    pub async fn fork_doc(&self) -> Automerge {
        let automerge = self.inner.lock().await;
        automerge.fork()
    }

    pub async fn merge_doc(&self, doc: &mut Automerge) -> Result<()> {
        let mut automerge = self.inner.lock().await;
        automerge.merge(doc)?;
        self.doc_update.send(automerge.clone())?;
        Ok(())
    }

    async fn send_msg(msg: Protocol, send: &mut SendStream) -> Result<()> {
        let encoded = postcard::to_stdvec(&msg)?;
        send.write_all(&(encoded.len() as u64).to_le_bytes())
            .await?;
        send.write_all(&encoded).await?;
        Ok(())
    }

    async fn recv_msg(recv: &mut RecvStream) -> Result<Protocol> {
        let mut incoming_len = [0u8; 8];
        recv.read_exact(&mut incoming_len).await?;
        let len = u64::from_le_bytes(incoming_len);

        let mut buffer = vec![0u8; len as usize];
        recv.read_exact(&mut buffer).await?;
        let msg: Protocol = postcard::from_bytes(&buffer)?;
        Ok(msg)
    }

    // initiate_syncの挙動（A: 送信者、B: 受信者）:
    // A側:

    // ローカルドキュメントのコピーを作成
    // 同期メッセージを生成し、Bに送信
    // Bからの応答を待機し、受信したメッセージを処理
    // 新しい変更をローカルドキュメントにマージ
    // 両方が完了するまでこのプロセスを繰り返す
    pub async fn initiate_sync(
        self: Arc<Self>,
        conn: iroh::net::endpoint::Connection,
    ) -> Result<()> {
        let (mut send, mut recv) = conn.open_bi().await?;

        let mut doc = self.fork_doc().await;
        let mut sync_state = sync::State::new();

        let mut is_local_done = false;
        loop {
            let msg = match doc.generate_sync_message(&mut sync_state) {
                Some(msg) => Protocol::SyncMessage(msg.encode()),
                None => Protocol::Done,
            };

            if !is_local_done {
                is_local_done = matches!(msg, Protocol::Done);
                Self::send_msg(msg, &mut send).await?;
            }

            let msg = Self::recv_msg(&mut recv).await?;
            let is_remote_done = matches!(msg, Protocol::Done);

            // process incoming message
            if let Protocol::SyncMessage(sync_msg) = msg {
                let sync_msg = sync::Message::decode(&sync_msg)?;
                doc.receive_sync_message(&mut sync_state, sync_msg)?;
                self.merge_doc(&mut doc).await?;
            }

            if is_remote_done && is_local_done {
                // both sides are done
                break;
            }
        }

        send.finish()?;

        Ok(())
    }

    // respond_syncの挙動:
    // B側:

    // 接続を受け入れ
    // Aからのメッセージを待機
    // 受信したメッセージを処理し、ローカルドキュメントを更新
    // 応答メッセージを生成してAに送信
    // 両方が完了するまでこのプロセスを繰り返す
    pub async fn respond_sync(
        self: Arc<Self>,
        conn: iroh::net::endpoint::Connecting,
    ) -> Result<()> {
        let (mut send, mut recv) = conn.await?.accept_bi().await?;

        let mut doc = self.fork_doc().await;
        let mut sync_state = sync::State::new();

        let mut is_local_done = false;
        loop {
            let msg = Self::recv_msg(&mut recv).await?;
            let is_remote_done = matches!(msg, Protocol::Done);

            // process incoming message
            if let Protocol::SyncMessage(sync_msg) = msg {
                let sync_msg = sync::Message::decode(&sync_msg)?;
                doc.receive_sync_message(&mut sync_state, sync_msg)?;
                self.merge_doc(&mut doc).await?;
            }

            let msg = match doc.generate_sync_message(&mut sync_state) {
                Some(msg) => Protocol::SyncMessage(msg.encode()),
                None => Protocol::Done,
            };

            if !is_local_done {
                is_local_done = matches!(msg, Protocol::Done);
                Self::send_msg(msg, &mut send).await?;
            }

            if is_remote_done && is_local_done {
                // both sides are done
                break;
            }
        }

        send.finish()?;

        Ok(())
    }

    pub async fn send_user_input(&self, key: String, value: String) -> Result<()> {
        let mut doc = self.inner.lock().await;
        let mut t = doc.transaction();
        t.put(automerge::ROOT, key.clone(), value.clone())?;
        t.commit();
        self.user_input.send((key, value))?;
        self.doc_update.send(doc.clone())?;
        Ok(())
    }

    // pub async fn force_sync(&self, send: &mut SendStream) -> Result<()> {
    //     let doc = self.inner.lock().await;
    //     let changes = doc.get_changes(&[]);
    //     let sync_message = sync::Message {
    //         heads: doc.get_heads(),
    //         need: vec![],
    //         have: vec![],
    //         changes: changes.into(),
    //         version: sync::MessageVersion::default(),
    //         supported_capabilities: sync::Capabilities::default(),
    //     };
    //     let msg = Protocol::SyncMessage(sync_message.encode());
    //     Self::send_msg(msg, send).await?;
    //     Ok(())
    // }

    pub async fn handle_connection(
        self: Arc<Self>,
        mut send: SendStream,
        mut recv: RecvStream,
    ) -> Result<()> {
        let mut doc = self.fork_doc().await;
        let mut sync_state = sync::State::new();
        let mut user_input_rx = self.user_input.subscribe();

        info!("Starting handle_connection");
        // self.force_sync(&mut send).await?;

        loop {
            tokio::select! {
                msg = Self::recv_msg(&mut recv) => {
                    match msg? {
                        Protocol::SyncMessage(sync_msg) => {
                            debug!("Received SyncMessage");
                            let sync_msg = sync::Message::decode(&sync_msg)?;
                            doc.receive_sync_message(&mut sync_state, sync_msg)?;
                            self.merge_doc(&mut doc).await?;
                        }
                        Protocol::UserInput { key, value } => {
                            info!("Received UserInput: {} = {}", key, value);
                            let mut t = doc.transaction();
                            t.put(automerge::ROOT, key, value)?;
                            t.commit();
                            self.merge_doc(&mut doc).await?;
                        }
                        Protocol::Done => {
                            info!("Received Done message");
                            break;
                        }
                    }
                }
                Ok((key, value)) = user_input_rx.recv() => {
                    info!("Sending UserInput: {} = {}", key, value);
                    Self::send_msg(Protocol::UserInput { key, value }, &mut send).await?;
                }
            }

            if let Some(msg) = doc.generate_sync_message(&mut sync_state) {
                debug!("Sending SyncMessage");
                Self::send_msg(Protocol::SyncMessage(msg.encode()), &mut send).await?;
            }
        }

        info!("Ending handle_connection");
        self.sync_finished.send(self.fork_doc().await).await?;
        Ok(())
    }
}

impl ProtocolHandler for IrohAutomergeProtocol {
    fn accept(
        self: Arc<Self>,
        conn: iroh::net::endpoint::Connecting,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>> {
        Box::pin(async move {
            let (send, recv) = conn.await?.accept_bi().await?;
            Arc::clone(&self).handle_connection(send, recv).await
        })
    }
}
