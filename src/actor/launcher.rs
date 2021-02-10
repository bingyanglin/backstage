use std::fmt::Display;

use futures::future::AbortHandle;

use super::*;

pub trait App<A: 'static + Clone + Send + Sync>: 'static + Actor<A> + Addressable<A> {}

impl<T, A> App<A> for T
where
    T: 'static + Actor<A> + Addressable<A>,
    A: 'static + Clone + Send + Sync,
{
}

#[async_trait]
pub trait Launcher<A>: Send
where
    A: 'static + Clone + Send + Sync,
{
    fn get_main_sender(&self) -> &UnboundedSender<Event<A>>;
    fn get_main_receiver(&mut self) -> &mut UnboundedReceiver<Event<A>>;
    fn get_service(&mut self) -> &mut Service;
    fn get_address(&self) -> &A;
    fn add_app<T: App<A>>(self, app: T) -> Self;
    fn start_app(&mut self, app: A);
    fn shutdown_app(&mut self, app: A);
    fn exit_program(&mut self, ctrl_c: bool);
    fn get_all_apps(&mut self) -> &mut Vec<Box<dyn App<A>>>;
}

#[async_trait]
impl<T, A> Actor<A> for T
where
    T: Launcher<A> + Addressable<A>,
    A: 'static + Clone + Send + Sync + Eq + Display,
{
    async fn init(&mut self, supervisor: Option<A>) -> NeedResult {
        tokio::spawn(ctrl_c(self.get_main_sender().clone()));
        // info!("Initializing {} Apps", self.app_count);
        // update service to be Initializing
        self.get_service().update_status(ServiceStatus::Initializing);
        // tell active apps
        // self.launcher_status_change();
        Ok(())
    }

    async fn event_loop(&mut self, supervisor: Option<A>) -> NeedResult {
        // update service to be Running
        self.get_service().update_status(ServiceStatus::Running);
        // tell active apps
        // self.launcher_status_change();
        let address = self.address();
        let mut handles = self
            .get_all_apps()
            .drain(..)
            .map(|app| {
                let (abort_handle, abort_registration) = AbortHandle::new_pair();
                let join_handle = tokio::spawn(app.start_abortable(abort_registration, Some(address.clone())));
                (abort_handle, join_handle)
            })
            .collect::<Vec<_>>();
        loop {
            let event = self.get_main_receiver().recv().await;
            if let Some(event) = event {
                match event {
                    Event::StartApp(address) => {
                        info!("Starting app with address {}", address);
                        self.start_app(address.clone());
                    }
                    Event::ShutdownApp(address) => {
                        info!("Shutting down app with address {}", address);
                        self.shutdown_app(address.clone());
                    }
                    Event::StatusChange(service) => {
                        info!("{} is {:?}, telling all active applications", service.name, service.status);
                        // self.status_change(service);
                    }
                    Event::ExitProgram { using_ctrl_c } => {
                        if !self.get_service().is_stopping() {
                            info!("Exiting application");
                            // update service to be Stopping
                            self.get_service().update_status(ServiceStatus::Stopping);
                            // tell active apps
                            // self.launcher_status_change();
                        }
                        // identify if ctrl_c has been consumed
                        // if using_ctrl_c {
                        //    self.consumed_ctrl_c = true;
                        // }
                        for (abort_handle, ref mut join_handle) in handles.iter_mut().rev() {
                            abort_handle.abort();
                            // join_handle.abort();
                            match join_handle.await {
                                Ok(source) => {
                                    info!("ResultSource: {:?}", source);
                                }
                                Err(e) => {
                                    error!("JoinError: {}", e);
                                }
                            }
                        }
                        return Err(Need::Abort);
                    }
                    Event::RequestService(ref address) => {
                        if address == self.get_address() {
                            let service = self.get_service().clone();
                            self.get_main_sender().send(Event::StatusChange(service)).map_err(|_| Need::Abort)?;
                        } else {
                            // let res = self
                            //    .send(address.clone(), Event::RequestService(address))
                            //    .await
                            //    .map_err(|_| Need::Abort)?;
                        }
                    }
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    async fn teardown(&mut self, status: ResultSource, supervisor: Option<A>) -> NeedResult {
        match status {
            ResultSource::Init(res) | ResultSource::Loop(res) | ResultSource::Teardown(res) => {}
        }
        // update service to stopped
        self.get_service().update_status(ServiceStatus::Stopped);
        info!("Goodbye");
        Ok(())
    }

    fn aborted(&self, aborted: Aborted, supervisor: Option<A>) {
        todo!()
    }

    fn timed_out(&self, elapsed: Elapsed, supervisor: Option<A>) {
        todo!()
    }
}

/// Useful function to exit program using ctrl_c signal
async fn ctrl_c<A>(sender: UnboundedSender<Event<A>>)
where
    A: 'static + Clone + Send + Sync,
{
    // await on ctrl_c
    tokio::signal::ctrl_c().await.unwrap();
    // exit program using launcher
    let _ = sender.send(Event::ExitProgram { using_ctrl_c: true });
}
