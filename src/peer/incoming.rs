use crate::{ import::*, * };

/// Type representing Messages coming in over the wire, for internal use only.
//
pub(super) struct Incoming
{
	pub(crate) msg: Result<WireFormat, ThesRemoteErr>
}

impl Message for Incoming
{
	type Return = ();
}


/// Handler for incoming messages
//
impl Handler<Incoming> for Peer
{
fn handle( &mut self, incoming: Incoming ) -> Return<'_, ()>
{

async move
{
	let cid_null = ConnID::null();

	let frame = match incoming.msg
	{
		Ok ( mesg  ) => mesg,
		Err( error ) =>
		{
			error!( "Error extracting MultiService from stream: {:#?}", error );

			self.pharos.send( PeerEvent::Error( ConnectionError::Deserialize ) ).await.expect( "pharos not closed" );

			// Send an error back to the remote peer and close the connection
			//
			self.send_err( cid_null, &ConnectionError::Deserialize, true ).await;

			return
		}
	};


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
	//                 if it's null, it means the message is meant for this peer (ConnectionError).
	//
	// (leaves distinguishing between send/call/response/error)
	//
	// CID   absent  -> Send
	// CID   unknown -> Call
	//
	// CID   present -> Return/Error
	//
	// (leaves Return/Error)
	//
	// sid null      -> ConnectionError
	//
	// I think these should never fail, because they accept random data in the current implementation.
	// However, since it's implementation dependant, and we are generic, we can't know that. It's probably
	// safer to assume that if these do fail we close the connection because all bet's are off for following
	// messages.
	//
	// TODO: This next block is what requires that MS is Sync. I don't understand why. It says there is a
	// requirement of Send for &MS. Probably it's the call frame.service() which takes &self.
	//
	let sid = match frame.service()
	{
		Ok ( sid ) => sid,
		Err( err ) =>
		{
			error!( "Fail to get service_id from incoming frame: {}", err );

			self.pharos.send( PeerEvent::Error( ConnectionError::Deserialize ) ).await.expect( "pharos not closed" );

			// Send an error back to the remote peer and close the connection
			//
			self.send_err( cid_null, &ConnectionError::Deserialize, true ).await;

			return
		}
	};


	// Deserialize connection id
	//
	let cid = match frame.conn_id()
	{
		Ok ( cid ) => cid,
		Err( err ) =>
		{
			error!( "Fail to get conn_id from incoming frame: {}", err );

			self.pharos.send( PeerEvent::Error( ConnectionError::Deserialize ) ).await.expect( "pharos not closed" );

			// Send an error back to the remote peer and close the connection
			//
			self.send_err( cid_null, &ConnectionError::Deserialize, true ).await;

			return
		}
	};


	// It's a connection error from the remote peer
	//
	if sid.is_null()
	{
		remote_conn_err( self, frame.mesg(), cid ).await;
	}


	// it's an incoming send
	//
	else if cid.is_null()
	{
		incoming_send( self, sid, cid_null, frame ).await;
	}


	// it's a succesful response to a (relayed) call
	//
	else if let Some( channel ) = self.responses.remove( &cid )
	{
		// It's a response
		//
		trace!( "Incoming Return" );

		// TODO: verify our error handling story here. Normally if this
		// fails it means the receiver of the channel was dropped... so
		// they are no longer interested in the reponse? Should we log?
		// Have a different behaviour in release than debug?
		//
		let _ = channel.send( Ok( frame ) );
	}


	// it's a call (!cid.is_null() and cid is unknown)
	//
	else
	{
		incoming_call( self, cid, sid, frame ).await;
	}

}.boxed() // End of async move

} // end of handle
} // end of impl Handler



// It's a connection error from the remote peer
//
// This includes failing to deserialize our messages, failing to relay, unknown service, ...
// TODO: when we sent a Call, it will have the cid in frame, so we should correctly react
// to that and forward it to the original caller.
//
async fn remote_conn_err( peer: &mut Peer, msg: Bytes, cid: ConnID )
{
	// We can correctly interprete the error
	//
	if let Ok( err ) = serde_cbor::from_slice::<ConnectionError>( &msg )
	{
		// TODO: Do we want to log here if we relay? Maybe only debug
		//
		error!( "Remote error: {:?}", err );

		// Notify observers
		//
		let shine = PeerEvent::RemoteError( err.clone() );
		peer.pharos.send( shine ).await.expect( "pharos not closed" );


		// We need to report the connection error to the caller
		//
		if let Some( channel ) = peer.responses.remove( &cid )
		{
			// If this returns an error, it means the receiver was dropped, so if they no longer
			// care for the result, neither do we, so ignoring the result.
			//
			let _ = channel.send( Err( err ) );
		}
	}

	// TODO
	//
	else
	{
		error!( "We received an error message from a remote peer, but couldn't deserialize it" );

		unimplemented!();
	}
}



async fn incoming_send
(
	peer    : &mut Peer        ,
	sid     : ServiceID        ,
	cid_null: ConnID           ,
	frame   : WireFormat ,
)
{
	trace!( "Incoming Send" );


	if let Some( typeid ) = peer.services.get( &sid )
	{
		trace!( "Incoming Send for local Actor" );

		// We are keeping our internal state consistent, so the unwrap is fine. if it's in
		// peer.services, it's in peer.service_maps.
		//
		let sm = &mut peer.service_maps.get( &typeid ).unwrap();

		match sm.send_service( frame )
		{
			Ok(fut) =>
			{
				// TODO: we should do error handling when the send fails.
				//
				let res = peer.exec.spawn( async{ let _ = fut.await; } );

				if res.is_err()
				{
					peer.pharos.send( PeerEvent::Error( ConnectionError::InternalServerError ) ).await

						.expect( "pharos not closed" )
					;

					// Send an error back to the remote peer and close the connection
					//
					peer.send_err( cid_null, &ConnectionError::InternalServerError, false ).await;
				}
			}

			Err(e) =>
			{
				error!( "Failed to send message to handler for service [{:?}]: {:?}", sid, e );

				match e
				{
					ThesRemoteErr::Deserialize(..) =>
					{
						peer.pharos.send( PeerEvent::Error( ConnectionError::Deserialize ) ).await.expect( "pharos not closed" );

						// Send an error back to the remote peer and close the connection
						//
						peer.send_err( cid_null, &ConnectionError::Deserialize, true ).await;
					},


					ThesRemoteErr::Downcast(..) =>
					{
						peer.pharos.send( PeerEvent::Error( ConnectionError::InternalServerError ) ).await

							.expect( "pharos not closed" )
						;

						// Send an error back to the remote peer and close the connection
						//
						peer.send_err( cid_null, &ConnectionError::InternalServerError, false ).await;
					},


					ThesRemoteErr::UnknownService(..) =>
					{
						let err = ConnectionError::UnknownService( Into::<Bytes>::into( sid ).to_vec() );

						// Send an error back to the remote peer and don't close the connection
						//
						peer.send_err( cid_null, &err, false ).await;

						let evt = PeerEvent::Error( err );
						peer.pharos.send( evt ).await.expect( "pharos not closed" );

					},

					_ => {}
				}
			}
		}
	}


	// service_id in peer.relay => Send to recipient found in peer.relay.
	//
	else if let Some( relay_id ) = peer.relayed.get( &sid )
	{
		trace!( "Incoming Send for relayed Actor" );

		// We are keeping our internal state consistent, so the unwrap is fine. if it's in
		// peer.relayed, it's in peer.relays.
		//
		let relay = &mut peer.relays.get_mut( &relay_id ).unwrap().0;

		// if this fails, well, the peer is no longer there. Warn remote and observers.
		// We let remote know we are no longer relaying to this service.
		// TODO: should we remove it from peer.relayed? Normally there is detection mechanisms
		//       already that should take care of this, but then if they work, we shouldn't be here...
		//       also, if we no longer use unbounded channels, this might fail because the
		//       channel is full.
		//
		if  relay.send( frame ).await.is_err()
		{
			let err = ConnectionError::FailedToRelay( Into::<Bytes>::into( sid ).to_vec() );
			error!( "Lost relay: {:?}", err );

			peer.send_err( cid_null, &err, false ).await;

			let err = PeerEvent::Error( err );
			peer.pharos.send( err ).await.expect( "pharos not closed" );
		};
	}


	// service_id unknown => send back and log error
	//
	else
	{
		error!( "Incoming Send for unknown Service: {}", &sid );

		// Send an error back to the remote peer and to the observers
		//
		let err = ConnectionError::UnknownService( Into::<Bytes>::into( sid ).to_vec() );
		peer.send_err( cid_null, &err, false ).await;

		let err = PeerEvent::Error( err );
		peer.pharos.send( err ).await.expect( "pharos not closed" );
	}
}



async fn incoming_call
(
	peer    : &mut Peer  ,
	cid     : ConnID     ,
	sid     : ServiceID  ,
	frame   : WireFormat ,
)
{
	trace!( "Incoming Call" );

	let self_addr = match peer.addr
	{
		Some(ref addr) => addr,

		// we no longer have our address, we're shutting down. we can't really do anything
		// without our address we won't have the sink for the connection either. We can
		// no longer send outgoing messages
		//
		None => return,
	};


	// It's a call for a local actor
	//
	if let Some( typeid ) = peer.services.get( &sid )
	{
		trace!( "Incoming Call for local Actor" );

		// We are keeping our internal state consistent, so the unwrap is fine. if it's in
		// peer.services, it's in peer.service_maps.
		//
		let sm = &mut peer.service_maps.get( &typeid ).unwrap();

		// Call actor
		//
		match sm.call_service( frame, self_addr.recipient() )
		{
			Ok(fut) =>
			{
				// TODO: we should do error handling when the call fails.
				//
				let res = peer.exec.spawn( async{ let _ = fut.await; } );

				if res.is_err()
				{
					peer.pharos.send( PeerEvent::Error( ConnectionError::InternalServerError ) ).await

						.expect( "pharos not closed" )
					;

					// Send an error back to the remote peer and close the connection
					//
					peer.send_err( cid, &ConnectionError::InternalServerError, false ).await;
				}
			}

			Err(e) =>
			{
				error!( "Failed to call handler for service [{:?}]: {:?}", sid, e );

				match e
				{
					ThesRemoteErr::Deserialize(..) =>
					{
						peer.pharos.send( PeerEvent::Error( ConnectionError::Deserialize ) ).await

							.expect( "pharos not closed" );

						// Send an error back to the remote peer and close the connection
						//
						peer.send_err( cid, &ConnectionError::Deserialize, true ).await;
					},


					ThesRemoteErr::UnknownService(..) =>
					{
						let err = ConnectionError::UnknownService( Into::<Bytes>::into( sid ).to_vec() );

						// Send an error back to the remote peer and don't close the connection
						//
						peer.send_err( cid, &err, false ).await;

						peer.pharos.send( PeerEvent::Error( err ) ).await.expect( "pharos not closed" );
					},


					ThesRemoteErr::Downcast(..) =>
					{
						peer.pharos.send( PeerEvent::Error( ConnectionError::InternalServerError ) ).await

							.expect( "pharos not closed" )
						;

						// Send an error back to the remote peer and close the connection
						//
						peer.send_err( cid, &ConnectionError::InternalServerError, false ).await;
					},


					_ => {}
				}
			}
		}
	}


	// Call for relayed actor
	//
	// service_id in peer.relay   => Create Call and send to recipient found in peer.relay.
	//
	// What could possibly go wrong???
	// - relay peer has been shut down (eg. remote closed connection)
	// - we manage to call, but then when we await the response, the relay goes down, so the
	//   sender of the channel for the response will come back as a disconnected error.
	// - the remote peer responds with a ConnectionError
	//
	else if let Some( relayed ) = peer.relayed.get( &sid )
	{
		trace!( "Incoming Call for relayed Actor" );

		// The unwrap is safe, because we just checked peer.relayed and we shall keep both
		// in sync
		//
		let mut relayed   = peer.relays.get_mut( &relayed ).unwrap().0.clone();
		let mut self_addr = self_addr.clone();

		peer.exec.spawn( async move
		{
			// TODO: until we have bounded channels, this should never fail, so I'm leaving the expect.
			//
			let called = relayed.call( Call::new( frame ) ).await.expect( "Call to relay failed" );


			// TODO: Let the incoming remote know that their call failed
			// not sure the current process wants to know much about this. We sure shouldn't
			// crash.
			//
			match called
			{
				// Sending out over the sink (connection) didn't fail
				//
				Ok( receiver ) =>	{ match receiver.await
				{
					// The channel was not dropped before resolving, so the relayed connection didn't close
					// until we got a response.
					//
					Ok( result ) =>
					{
						match result
						{
							// The remote managed to process the message and send a result back
							// (no connection errors like deserialization failures etc)
							//
							Ok( resp ) =>
							{
								trace!( "Got response from relayed call, sending out" );

								// TODO: until we have bounded channels, this should never fail, so I'm leaving the expect.
								//
								self_addr.send( resp ).await.expect( "Failed to send response from relay out on connection" );
							},

							// The relayed remote had connection errors, forward the error to the caller.
							// This is a particular case. In principle we can imagine that the caller does not know
							// whether we are providing a service, or relaying it. However. When these errors would
							// happen here, on our connection, it would mean the stream was potentially corrupt
							// and we would close the connection. However, as these errors are on the connection to the
							// relay, we don't need to close the connection. Currently we don't explicitly have an error
							// type to tell the caller that this is an error in a relayed process, but they might observe
							// that we don't close the connection.
							//
							Err(e) =>
							{
								warn!( "Got ERROR back from relayed call, error to caller: {:?}", &e );

								let err = Peer::prep_error( cid, &e );

								// TODO: until we have bounded channels, this should never fail, so I'm leaving the expect.
								//
								self_addr.send( err ).await.expect( "send msg to self" );
							},
						}
					},


					// This can only happen if the sender got dropped. Eg, if the remote relay goes down
					// Inform peer that their call failed because we lost connection to the relay after
					// it was sent out.
					//
					Err( e ) =>
					{
						error!( "Lost relay: {:?}", e );

						// Send an error back to the remote peer
						//
						let err = Peer::prep_error( cid, &ConnectionError::LostRelayBeforeResponse );


						// TODO: until we have bounded channels, this should never fail, so I'm leaving the expect.
						//
						self_addr.send( err ).await.expect( "send msg to self" );
					}
				}},

				// Sending out call to relayed failed. This normally only happens if the connection
				// was closed, or a network transport malfunctioned.
				//
				Err( e ) =>
				{
					error!( "Relay returned a ConnectionError for a remote Call: {:?}", &e );

					// Send an error back to the remote peer
					// We do not include
					//
					let err = Peer::prep_error( cid, &ConnectionError::FailedToRelay( Into::<Bytes>::into( sid ).to_vec() ) );


					// TODO: until we have bounded channels, this should never fail, so I'm leaving the expect.
					//
					self_addr.send( err ).await.expect( "send msg to self" );
				}
			}

		}).expect( "failed to spawn" );

	}

	// service_id unknown => send back and log error
	//
	else
	{
		error!( "Incoming Call for unknown Actor: {}", sid );

		// Send an error back to the remote peer and to the observers
		//
		let err = ConnectionError::UnknownService( Into::<Bytes>::into( sid ).to_vec() );
		peer.send_err( cid, &err, false ).await;

		let err = PeerEvent::Error( err );
		peer.pharos.send( err ).await.expect( "pharos not closed" );
	}

}
