pub use
{
	log          :: { *                                                          } ,
	thespis      :: { *                                                          } ,
	thespis_impl :: { *, remote::*, service_map, runtime::{ rt, tokio::TokioRT } } ,
	serde        :: { Serialize, Deserialize, de::DeserializeOwned               } ,
	std          :: { net::SocketAddr                                            } ,
	pharos       :: { Observable                                                 } ,


	futures      ::
	{
		channel :: { mpsc                                                    } ,
		future  :: { FutureExt                                               } ,
		stream  :: { Stream, StreamExt                                       } ,
		compat  :: { Future01CompatExt, Compat01As03, Compat01As03Sink, Stream01CompatExt, Sink01CompatExt } ,
	},


	tokio::
	{
		prelude :: { StreamAsyncExt, Stream as TokStream, stream::{ SplitStream as TokSplitStream, SplitSink as TokSplitSink } } ,
 		net     :: { TcpStream, TcpListener                                                                                    } ,
		codec   :: { Decoder, Framed                                                                                           } ,
	},
};



pub type TheSink = Compat01As03Sink<TokSplitSink<Framed<TcpStream, MulServTokioCodec<MS>>>, MS> ;
pub type MS      = MultiServiceImpl<ServiceID, ConnID, Codecs>                                  ;
pub type MyPeer  = Peer<TheSink, MS>                                                            ;

#[ derive( Serialize, Deserialize, Debug ) ] pub struct ServiceA  { pub msg : String }
#[ derive( Serialize, Deserialize, Debug ) ] pub struct ServiceB  { pub msg : String }


#[ derive( Serialize, Deserialize, Debug ) ]
//
pub struct ReturnA { pub resp: String }


impl Message for ServiceA { type Return = ReturnA; }
impl Message for ServiceB { type Return = ()       ; }



service_map!
(
	namespace:     peer_a   ;
	peer_type:     MyPeer   ;
	multi_service: MS       ;
	services     :

		ServiceB ,
		ServiceA ,
	;
);



// compiler bug
//
#[ allow( dead_code ) ]
//
pub async fn listen_tcp( socket: &str ) ->

	(TokSplitSink<Framed<TcpStream, MulServTokioCodec<MS>>>, TokSplitStream<Framed<TcpStream, MulServTokioCodec<MS>>>)

{
	// create tcp server
	//
	let socket   = socket.parse::<SocketAddr>().unwrap();
	let listener = TcpListener::bind( &socket ).expect( "bind address" );

	let codec: MulServTokioCodec<MS> = MulServTokioCodec::new();

	let stream = listener.incoming().take(1).into_future().compat().await
		.expect( "find one stream" ).0
		.expect( "find one stream" );

	codec.framed( stream ).split()
}



// compiler bug
//
#[ allow( dead_code ) ]
//
pub async fn connect_to_tcp( socket: &str ) -> (Addr<MyPeer>, mpsc::Receiver<PeerEvent>)
{
	// Connect to tcp server
	//
	let socket = socket.parse::<SocketAddr>().unwrap();
	let stream = TcpStream::connect( &socket ).compat().await.expect( "connect address" );

	// frame the connection with codec for multiservice
	//
	let codec: MulServTokioCodec<MS> = MulServTokioCodec::new();

	let (sink_a, stream_a) = codec.framed( stream ).split();

	// Create mailbox for peer
	//
	let mb  : Inbox<MyPeer> = Inbox::new()             ;
	let addr                = Addr ::new( mb.sender() );

	// create peer with stream/sink + service map
	//
	let mut peer = Peer::new( addr.clone(), stream_a.compat(), sink_a.sink_compat() ).expect( "spawn peer" );

	let evts = peer.observe( 10 );

	mb.start( peer ).expect( "Failed to start mailbox" );

	(addr, evts)
}

