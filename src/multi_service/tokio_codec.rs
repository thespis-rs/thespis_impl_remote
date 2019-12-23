//! The codec to frame the connection with our wire format.
//
use crate::{ import::*, MultiServiceImpl };
use super::HEADER_LEN;

/// The tokio codec to frame AsyncRead/Write streams.
/// TODO: test max_length
//
#[ derive( Debug ) ]
//
pub struct MulServTokioCodec
{
	max_length: usize  , // in bytes

}


impl MulServTokioCodec
{
	/// Create a new codec, with the max length of a single message in bytes. Note that this includes the
	/// header of the wireformat. For [`thespis_remote_impl::MultiServiceImpl`] the header is 36 bytes.
	///
	/// Setting a value to high might lead to more memory usage and could enable OOM/DDOS attacks.
	//
	pub fn new( max_length: usize ) -> Self
	{
		Self{ max_length }
	}
}


impl Decoder for MulServTokioCodec
{
	type Item  = MultiServiceImpl ;
	type Error = ThesRemoteErr    ;

	fn decode( &mut self, buf: &mut BytesMut ) -> Result< Option<Self::Item>, Self::Error >
	{
		trace!( "Decoding incoming message: {:?}", &buf );

		// Minimum length of an empty message is the u64 indicating the length + the header (sid/connid)
		//
		if buf.len() < 8 + HEADER_LEN  { return Ok( None ) }


		// parse the first 8 bytes to find out the total length of the message
		//
		let mut len = buf[..8]

			.as_ref()
			.read_u64::<LittleEndian>()
			.map_err( |_| ThesRemoteErr::Deserialize( "Tokio codec: Length".to_string() ))?

			as usize
		;


		// respect the max_length
		// TODO: does max length include the HEADER? Document and make consistent with Encoder.
		//
		if len > self.max_length
		{
			return Err( ThesRemoteErr::MessageSizeExceeded
			(
				format!( "Tokio Codec Decoder: max_length={:?} bytes, message={:?} bytes", self.max_length, len )

			).into())
		}

		// Message hasn't completely arrived yet
		//
		if buf.len() < len { return Ok( None ) }


		// We have an entire message.
		// Remove the length header, we won't need it anymore
		//
		buf.advance( 8 );
		len -= 8;

		// Consume the message
		//
		let buf = buf.split_to( len );

		// Convert
		//
		Ok( Some( MultiServiceImpl::try_from( Bytes::from( buf ) )? ) )
	}
}


// TODO: zero copy encoding. Currently we copy bytes in the output buffer.
// In principle we would like to not have to serialize the inner message
// before having access to this buffer.
//
impl Encoder for MulServTokioCodec
{
	type Item  = MultiServiceImpl ;
	type Error = ThesRemoteErr    ;

	fn encode( &mut self, item: Self::Item, buf: &mut BytesMut ) -> Result<(), Self::Error>
	{
		// respect the max_length
		//
		if item.len() > self.max_length
		{
			return Err( ThesRemoteErr::MessageSizeExceeded
			(
				format!( "Tokio Codec Encoder: max_length={:?} bytes, message={:?} bytes", self.max_length, item.len() )

			).into() )
		}


		let len = item.len() + 8;
		buf.reserve( len );

		let mut wtr = vec![];
		wtr.write_u64::<LittleEndian>( len as u64 ).expect( "Tokio codec encode: Write u64 to vec" );

		buf.put( wtr                         );
		buf.put( Into::<Bytes>::into( item ) );

		Ok(())
	}
}


#[ cfg(test) ]
//
mod tests
{
	// Tests:
	//
	// 1. A valid MultiServiceImpl, encoded + decoded should be identical to original.
	// 2. Send like 2 and a half full objects, test that 2 correctly come out, and there is the
	//    exact amount of bytes left in the buffer for the other half.
	// 3. TODO: send invalid data (not enough bytes to make a full multiservice header...)
	//
	use crate::{ * };
	use super::{ *, assert_eq };



	fn empty_data() -> MultiServiceImpl
	{
		let mut buf = BytesMut::with_capacity( 1 );
		buf.put( 0u8 );

		let m = MultiServiceImpl::create( ServiceID::from_seed( b"Empty Message" ), ConnID::random(), buf.into() );

		m
	}

	fn full_data() -> MultiServiceImpl
	{
		let mut buf = BytesMut::with_capacity( 5 );
		buf.put( b"hello".to_vec() );

		MultiServiceImpl::create( ServiceID::from_seed( b"Full Message" ), ConnID::random(), buf.into() )
	}


	#[test]
	//
	fn empty()
	{
		let mut codec = MulServTokioCodec::new( 1024 );
		let mut buf   = BytesMut::new();
		let     data  = empty_data();
		let     data2 = data.clone();

		codec.encode( data, &mut buf ).expect( "Encoding empty" );

		assert_eq!
		(
			data2,

			codec.decode( &mut buf )

				.expect( "No errors should occur"                      )
				.expect( "There should be some data (eg. Not Ok(None)" )
		);
	}


	#[test]
	//
	fn full()
	{
		let mut codec = MulServTokioCodec::new(1024);
		let mut buf   = BytesMut::new();
		let     data  = full_data();
		let     data2 = data.clone();

		codec.encode( data, &mut buf ).expect( "Encoding empty" );

		assert_eq!
		(
			data2,

			codec.decode( &mut buf )

				.expect( "No errors should occur"                      )
				.expect( "There should be some data (eg. Not Ok(None)" )
		);
	}


	#[test]
	//
	fn partials()
	{
		let mut codec: MulServTokioCodec = MulServTokioCodec ::new(1024);

		let mut buf = BytesMut::new();

		let     empty  = empty_data();
		let     full   = full_data ();

		let     empty2 = empty.clone();
		let     full2  = full .clone();
		let     full3  = full .clone();

		codec.encode( empty, &mut buf ).expect( "Encoding empty" ); // 41 bytes
		codec.encode( full , &mut buf ).expect( "Encoding full"  ); // 45 bytes
		codec.encode( full2, &mut buf ).expect( "Encoding full"  ); // 45 bytes

		// total is 131
		//
		assert_eq!( empty2, codec.decode( &mut buf ).expect( "Decode empty" ).expect( "Not None" ) );
		assert_eq!(  full3, codec.decode( &mut buf ).expect( "Decode empty" ).expect( "Not None" ) );
		assert_eq!( 45, buf.len() ); // there should be exactly 48 bytes sitting there waiting for the last.
	}
}
