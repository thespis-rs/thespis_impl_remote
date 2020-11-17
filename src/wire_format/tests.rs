//! Tests for the wire format:
//!
//! - Encoder
//!   -
//!
//! - Decoder
//!   ✔ send everything at once
//!   ✔ send data in small chunks, half the length, then the rest of the message in several parts.
//!   ✔ randomly intersperse Pending...needs this functionality in futures_ringbuf.
//!   - send incorrect data (eg. length) and verify the errors.
//!   - close connection halfway through and verify errors.
//!   - try to exceed the allowed length.
//!   - fuzz test
//!
use
{
	super           :: { *, assert_eq          } ,
	futures_ringbuf :: { Endpoint, Sketchy, Dictator } ,
	async_executors :: { AsyncStd              } ,
	futures         :: { task::LocalSpawnExt, join   } ,
};


pub trait MockConnection : FutAsyncRead + FutAsyncWrite + Send + Unpin {}

impl<T> MockConnection for T

	where T: FutAsyncRead + FutAsyncWrite + Send + Unpin

{}


#[ derive( Debug) ]
//
pub struct TestSuite<Wf, Si, St>

	where Wf: WireFormat                       ,
	      Si: Sink<Wf, Error=WireErr> + Send   ,
	      St: Stream<Item=Result<Wf, WireErr>> ,

{
	factory: fn( transport: Box<dyn MockConnection>, max_size: usize ) -> (Si, St),
	_phantom: PhantomData<Wf> ,
}


impl<Wf, Si, St> TestSuite<Wf, Si, St>

	where Wf: WireFormat                                       ,
	      Si: Sink<Wf, Error=WireErr> + Send + 'static + Unpin ,
	      St: Stream<Item=Result<Wf, WireErr>> + Unpin         ,

{
	/// Create a new test suite with the given function pointer that will frame the connection.
	//
	pub fn new( factory: fn( transport: Box<dyn MockConnection>, max_size: usize ) -> (Si, St) ) -> Self
	{
		Self { factory, _phantom: PhantomData }
	}

	/// Run all tests.
	//
	pub async fn run( &self )
	{
		self.send_all().await;
		self.send_chunked().await;
		self.read_pending().await;
	}


	pub async fn send_all( &self )
	{
		let (trans_a, trans_b) = Endpoint::pair( 64, 64 );
		let (mut sink_a, _  )  = (self.factory)( Box::new(trans_a), 64 );
		let (_, mut stream_b)  = (self.factory)( Box::new(trans_b), 64 );

		let sid = ServiceID::from_seed( &[1, 2, 3 ] );
		let cid = ConnID::random();
		let msg = "message".as_bytes();

		let mut wf = Wf::default();
		wf.set_sid( sid );
		wf.set_cid( cid );
		wf.write( msg ).expect( "be able to write serialized message" );

		let wf2 = wf.clone();

		sink_a.send( wf2 ).await.expect( "send on sink" );

		let received = stream_b.next().await.expect( "receive on stream" ).expect( "no WireErr");

		assert_eq!( received.len(), wf.len() );
		assert_eq!( received.sid(), wf.sid() );
		assert_eq!( received.sid(), sid      );
		assert_eq!( received.cid(), wf.cid() );
		assert_eq!( received.cid(), cid      );
		assert_eq!( received.msg(), wf.msg() );
		assert_eq!( received.msg(), msg      );
	}


	pub async fn send_chunked( &self )
	{
		// let _ = flexi_logger::Logger::with_str( "trace, thespis_remote=trace" ).start();

		let (trans_a, trans_b) = Endpoint::pair( 4, 4 );

		let (mut sink_a, _  ) = (self.factory)( Box::new(trans_a), 64 );
		let (_, mut stream_b) = (self.factory)( Box::new(trans_b), 64 );

		let sid = ServiceID::from_seed( &[1, 2, 3 ] );
		let cid = ConnID::random();
		let msg = "message".as_bytes();

		let mut wf = Wf::default();
		wf.set_sid( sid );
		wf.set_cid( cid );
		wf.write( msg ).expect( "be able to write serialized message" );

		let wf2 = wf.clone();

		debug!( "send_chunked: spawning sender" );

		AsyncStd.spawn_local( async move
		{
			sink_a.send( wf2 ).await.expect( "send on sink" );

		}).expect( "spawn");

		debug!( "send_chunked: waiting for stream" );

		let received = stream_b.next().await.expect( "receive on stream" ).expect( "no WireErr" );

		assert_eq!( received.len(), wf.len() );
		assert_eq!( received.sid(), wf.sid() );
		assert_eq!( received.sid(), sid      );
		assert_eq!( received.cid(), wf.cid() );
		assert_eq!( received.cid(), cid      );
		assert_eq!( received.msg(), wf.msg() );
		assert_eq!( received.msg(), msg      );
	}


	pub async fn read_pending( &self )
	{
		// let _ = flexi_logger::Logger::with_str( "trace, thespis_remote=trace" ).start();

		for _ in 0..2000
		{
			let seed = Dictator::seed();
			// let seed = 9493852723399469118;

			let (trans_a, trans_b) = Endpoint::pair( 64, 64 );
			let trans_b = Sketchy::new( trans_b, seed );

			let (mut sink_a, _  ) = (self.factory)( Box::new(trans_a), 64 );
			let (_, mut stream_b) = (self.factory)( Box::new(trans_b), 64 );

			let sid = ServiceID::from_seed( &[1, 2, 3 ] );
			let cid = ConnID::random();
			let msg = "message".as_bytes();

			let mut wf = Wf::default();
			wf.set_sid( sid );
			wf.set_cid( cid );
			wf.write( msg ).expect( "be able to write serialized message" );

			let wf2 = wf.clone();

			let (send, received) = join!
			(
				sink_a.send( wf2 ),
				stream_b.next(),
			);

			let received = received.expect( "some" ).expect( "no_error" );

			assert!( send.is_ok() );
			assert_eq!( received.len(), wf.len() );
			assert_eq!( received.sid(), wf.sid() );
			assert_eq!( received.sid(), sid      );
			assert_eq!( received.cid(), wf.cid() );
			assert_eq!( received.cid(), cid      );
			assert_eq!( received.msg(), wf.msg() );
			assert_eq!( received.msg(), msg      );
		}
	}
}
