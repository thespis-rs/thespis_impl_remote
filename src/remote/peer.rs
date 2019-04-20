use { crate :: { import::*, ThesError, runtime::rt, remote::ServiceID, remote::ConnID, single_thread::{ Addr, Rcpnt } } };

mod close_connection;
pub use close_connection::CloseConnection;

pub trait BoundsIn <MulService>: 'static + Stream< Item = Result<MulService, Error> > + Unpin {}
pub trait BoundsOut<MulService>: 'static + Sink<MulService, SinkError=Error> + Unpin          {}
pub trait BoundsMulService     : 'static + Message<Result=()> + MultiService                  {}

impl<T, MulService> BoundsIn<MulService> for T
where T: 'static + Stream< Item = Result<MulService, Error> > + Unpin {}

impl<T, MulService> BoundsOut<MulService> for T
where T: 'static + Sink<MulService, SinkError=Error> + Unpin {}

impl<T> BoundsMulService for T
where T: 'static + Message<Result=()> + MultiService {}


/// Represents a connection to another process over which you can send actor messages.
///
/// TODO: - let the user subscribe to connection close event.
///       - if you still have a recipient, so an address to this peer, but the remote end closes,
///         what happens when you send on the recipient (error handling in other words)
///
/// ### Closing the connection
///
/// The reasoning behind a peer is that it is tied to a stream/sink, often a framed connection.
/// When the connection closes for whatever reason, the peer should dissappear and no longer handle
/// any messages.
///
/// If the remote closes the connection, and you are no longer holding any addresses to this
/// peer (or recipients for remote actors), then the peer will get dropped.
///
/// If you do hold recipients and try to send on them, 2 things can happen. Since Send is like
/// throwing a message in a bottle, without feedback, it's infallible, so your message will
/// just get dropped silently. If you use call, which returns a result, you will get an error.
///
/// It's not yet implemented, but there will be an event that you will be able to subscribe to
/// to detect closed connections, so you can drop your recipients, try to reconnect, etc...
///
/// When the remote closes the connection, this peer will drop it's own adress, which will
///
//
pub struct Peer<Out, MulService>

	where Out        : BoundsOut<MulService> ,
	      MulService : BoundsMulService      ,
{
	outgoing      : Option< Out >             ,
	addr          : Option< Addr<Self> >      ,
	listen_handle : Option< RemoteHandle<()> >,

	services      : HashMap< <MulService as MultiService>::ServiceID , Box< Any > >                      ,
	local_sm      : HashMap< <MulService as MultiService>::ServiceID , Box< dyn ServiceMap<MulService>>> ,
	relay         : HashMap< <MulService as MultiService>::ServiceID , Addr<Self>   >                    ,
	responses     : HashMap< <MulService as MultiService>::ConnID    , oneshot::Sender<MulService> >     ,
}



impl<Out, MulService> Actor for Peer<Out, MulService>

	where Out        : BoundsOut<MulService> ,
	      MulService : BoundsMulService      ,
{
	fn started ( &mut self ) -> TupleResponse
	{
		async move
		{
			trace!( "Started Peer actor" );

		}.boxed()
	}


	fn stopped ( &mut self ) -> TupleResponse
	{
		async move
		{
			trace!( "Stopped Peer actor" );

		}.boxed()
	}
}



impl<Out, MulService> Peer<Out, MulService>

	where Out        : BoundsOut<MulService> ,
	      MulService : BoundsMulService      ,
{
	pub fn new( addr: Addr<Self>, incoming: impl BoundsIn<MulService>, outgoing: Out ) -> Self
	{
		let listen = Self::listen( addr.clone(), incoming );

		// Only way to able to close the connection... if it's tokio underneath. Anyways, since
		// we take any kind of sink and stream, it's probably best to specifically stop listening
		// when we want to close rather than count on drop side-effects.
		//
		// https://users.rust-lang.org/t/tokio-tcp-connection-not-closed-when-sender-is-dropped-futures-0-3-compat-layer/26910
		// https://github.com/tokio-rs/tokio/issues/852#issuecomment-459766144
		//
		let (remote, handle) = listen.remote_handle();
		rt::spawn( remote ).expect( "Failed to spawn listener" );

		Self
		{
			outgoing     : Some( outgoing ) ,
			addr         : Some( addr )     ,
			responses    : HashMap::new()   ,
			services     : HashMap::new()   ,
			local_sm     : HashMap::new()   ,
			relay        : HashMap::new()   ,
			listen_handle: Some( handle )   ,
		}
	}


	/// Register a handler for a service that you want to expose over this connection.
	///
	/// TODO: define what has to happen when called several times on the same service
	///       options: 1. error
	///                2. replace prior entry
	///                3. allow several handlers for the same service (not very likely)
	//
	pub fn register_service<Service: Message>( &mut self, sid: <MulService as MultiService>::ServiceID, sm: Box<dyn ServiceMap<MulService>>, handler: Box< Recipient<Service> > )
	{
		self.services.insert( sid.clone(), box Rcpnt::new( handler ) );
		self.local_sm.insert( sid        , sm                        );
	}


	pub fn register_relayed_service<Service: Message>( &mut self, sid: <MulService as MultiService>::ServiceID, peer: Addr<Self> )
	{
		self.relay.insert( sid.clone(), peer );
	}



	async fn listen( mut self_addr: Addr<Self>, mut incoming: impl BoundsIn<MulService> )
	{
		loop
		{
			let event: Option< Result< MulService, _ > > = await!( incoming.next() );

			trace!( "got incoming event on stream" );

			match event
			{
				Some( conn ) => { match conn
				{
					Ok ( mesg  ) =>
					{
						await!( self_addr.send( Incoming { mesg } ) ).expect( "Send to self in peer" );
					},

					Err( error ) =>
					{
						error!( "Error extracting MultiService from stream: {:#?}", error );

						// TODO: we should send an error to the remote before closing the connection.
						//       we should also close the sending end.
						//
						// return Err( ThesError::CorruptFrame.into() );
					}
				}},

				None =>
				{
					trace!( "Connection closed" );

					return await!( self_addr.send( CloseConnection ) ).expect( "Send Drop to self in Peer" );
				}
			};
		}
	}



	// actor in self.process => deserialize, use send on recipient
	// actor in self.relay   => forward
	// actor unknown         => send back and log error
	//
	async fn handle_send( &mut self, _frame: MulService )
	{

	}


	// actor in self.process => deserialize, use call on recipient,
	//                          when completes, reconstruct multiservice with connID for response.
	// actor in self.relay   => Create Call and send to recipient found in self.relay.
	// actor unknown         => send back and log error
	//
	async fn handle_call( &mut self, _frame: MulService )
	{

	}


	// actor in self.process => look in self.responses, deserialize and send response in channel.
	// actor in self.relay   => Create Call and send to recipient found in self.relay.
	// actor unknown         => send back and log error
	//
	async fn handle_response( &mut self, _frame: MulService )
	{

	}


	// log error?
	// allow user to set a handler for these...
	//
	async fn handle_error( &mut self, _frame: MulService )
	{

	}


	// actually send the message accross the wire
	//
	async fn send_msg( &mut self, msg: MulService ) -> ThesRes<()>
	{
		match &mut self.outgoing
		{
			Some( out ) => await!( out.send( msg ) )                             ,
			None        => Err( ThesError::PeerSendAfterCloseConnection.into() ) ,
		}
	}
}



// On an outgoing call, we need to store the conn_id and the peer to whome to return the response
//
// On outgoing call made by a local actor, store in the oneshot sender in self.responses
//
impl<Out, MulService> Handler<MulService> for Peer<Out, MulService>

	where Out        : BoundsOut<MulService> ,
	      MulService : BoundsMulService      ,

{
	fn handle( &mut self, msg: MulService ) -> Response<()>
	{
		async move
		{
			debug!( "Peer sending OUT" );

			let _ = await!( self.send_msg( msg ) );

		}.boxed()
	}
}



/// Type representing the outgoing call
//
pub struct Call<MulService: MultiService>
{
	mesg: MulService,
}

impl<MulService: 'static +  MultiService> Message for Call<MulService>
{
	type Result = ThesRes< oneshot::Receiver<MulService> >;
}

impl<MulService: MultiService> Call<MulService>
{
	pub fn new( mesg: MulService ) -> Self
	{
		Self{ mesg }
	}
}



/// Type representing Messages coming in over the wire
//
struct Incoming<MulService: MultiService>
{
	pub mesg: MulService,
}

impl<MulService: 'static + MultiService> Message for Incoming<MulService>
{
	type Result = ();
}



/// Handler for outgoing Calls
//
impl<Out, MulService> Handler<Call<MulService>> for Peer<Out, MulService>

	where Out: BoundsOut<MulService>,
	      MulService: BoundsMulService,
{
	fn handle( &mut self, call: Call<MulService> ) -> Response< <Call<MulService> as Message>::Result >
	{
		debug!( "peer: starting Handler<Call<MulService>>" );

		let (sender, receiver) = oneshot::channel::< MulService >();

		let conn_id = call.mesg.conn_id().expect( "Failed to get connection ID from call" );

		self.responses.insert( conn_id, sender );

		let fut = async move
		{
			await!( self.send_msg( call.mesg ) )?;

			// We run expect here, because we are holding the sender part of this, so
			// it should never be cancelled. It is more pleasant for the client
			// not to have to deal with 2 nested Results.
			//
			// TODO: There is one exeption, when this actor get's stopped, the client will
			// be holding this future and the expect will panic. That's bad, but we will
			// investigate how to deal with that once we have some more real life usecases.
			//
			// TODO: 2: We return a oneshot::Receiver, rather than a future, so we expose
			// implementation details to the user = BAD!!!
			//
			Ok( receiver )

		}.boxed();

		trace!( "peer: returning from call handler" );

		fut
	}
}



/// Handler for incoming messages
//
impl<Out, MulService> Handler<Incoming<MulService>> for Peer<Out, MulService>

	where Out       : BoundsOut<MulService>,
	      MulService: BoundsMulService     ,
{
	fn handle( &mut self, incoming: Incoming<MulService> ) -> Response<()>
	{
		debug!( "Incoming message!" );

		async move
		{
			let frame = incoming.mesg;

			// algorithm for incoming messages. Options are:
			//
			// 1. incoming send/call               for local/relayed/unknown actor (6 options)
			// 2.       response to outgoing call from local/relayed actor         (2 options)
			// 3. error response to outgoing call from local/relayed actor         (2 options)
			//
			// 4 possibilities with ServiceID and ConnID. These can be augmented with
			// predicates about our local state (sid in local table, routing table, unknown), + the codec
			// which gives us largely the 10 needed states:
			//
			// SID  present -> always tells us if it's for local/relayed/unknown actor
			//                 based on our routing tables
			//
			//                 if it's absent, it can come from our open connections table.
			//
			// (leaves distinguishing between send/call/response/error)
			//
			// CID   absent  -> Send
			// CID   unknown -> Call
			//
			// CID   present -> Response/Error
			//
			// (leaves Response/Error)
			//
			// both  present -> Error, if response, no need for SID since ConnID will be in our open connections table
			// none  present -> not supported?
			// codec present -> obligatory, not used to distinguish use case, but do we accept utf8 for errors?
			//                  maybe not, strongly typed errors defined by thespis encoded with cbor seem better.
			//
			// dbg!( &frame.conn_id().unwrap() );
			// dbg!( &frame.service().unwrap() );
			// dbg!( &frame.service().unwrap().is_null() );

			let sid = frame.service().expect( "fail to getting conn_id from frame"    );
			let cid = frame.conn_id().expect( "fail to getting service_id from frame" );

			// it's an incoming send
			//
			if cid.is_null()
			{
				debug!( "Incoming Send" );


				if let Some( handler ) = self.services.get( &sid )
				{
					debug!( "Incoming Send for local Actor" );


					self.local_sm

						.get( &sid ).expect( "failed to find service map." )
						.send_service( frame, handler )
					;
				}


				// service_id in self.relay   => Create Call and send to recipient found in self.relay.
				//
				else if let Some( r ) = self.relay.get_mut( &sid )
				{
					debug!( "Incoming Send for relayed Actor" );

					await!( r.send( frame ) ).expect( "Failed relaying send to other peer" );
				}

				// service_id unknown         => send back and log error
				//
				else
				{
					error!( "Incoming Call for unknown Actor" );

				}
			}


			// it's a call
			//
			else if !self.responses.contains_key( &cid ) // && !self.relayed_calls.contains_key( &cid )
			{
				debug!( "Incoming Call" );

				if let Some( ref addr ) = self.addr
				{
					// service_id in self.process => deserialize, use call on recipient, when completes,
					//                               reconstruct multiservice with connID for response.
					//
					if let Some( handler ) = self.services.get( &sid )
					{
						debug!( "Incoming Call for local Actor" );

						// Call actor
						//
						self.local_sm.get( &sid ).expect( "failed to find service map." )

							.call_service( frame, handler, addr.clone().recipient::<MulService>() );
					}


					// service_id in self.relay   => Create Call and send to recipient found in self.relay.
					//
					else if let Some( r ) = self.relay.get_mut( &sid )
					{
						debug!( "Incoming Call for relayed Actor" );

						let mut r    = r.clone();
						let mut addr = addr.clone();

						rt::spawn( async move
						{
							let channel = await!( r.call( Call::new( frame ) ) ).expect( "Call to relay failed" );

							let resp    = await!( channel.expect( "send call out over connection" ) )

								.expect( "failed to read from channel for response from relay" );

							trace!( "Got response from relayed call, sending out" );

							await!( addr.send( resp ) ).expect( "Failed to send response from relay out on connection" );

						}).expect( "failed to spawn" );

					}

					// service_id unknown         => send back and log error
					//
					else
					{
						trace!( "Incoming Call for unknown Actor" );


					}
				}

				else
				{
					// we no longer have our address, we're shutting down. We should prevent the caller somehow.
				}


			}

			// it's a response
			//
			else if let Some( channel ) = self.responses.remove( &cid )
			{
				debug!( "Incoming Response" );

				// TODO: verify our error handling story here. Normally if this
				// fails it means the receiver of the channel was dropped... so
				// they are no longer interested in the reponse? Should we log?
				// Have a different behaviour in release than debug?
				//
				let _ = channel.send( frame );
			}

			// it's a response for a relayed actor (need to keep track of relayed cid's)
			//
			// else if let Some( channel = self.responses.get( cid ))
			// {
			// 	trace!( "Incoming Response" );

			// }

			// it's an error
			//
			else
			{
				debug!( "Incoming Error" );

				await!( self.handle_error( frame ) )
			}

		}.boxed()
	}
}