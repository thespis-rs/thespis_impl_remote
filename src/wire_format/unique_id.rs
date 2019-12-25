use crate::{ import::*, ThesRemoteErr };


/// A unique identifier for a service that is exposed to other processes. This will allow
/// identifying the type to which the payload needs to be deserialized and the actor to which
/// this message is to be delivered.
///
/// Ideally we want to use 128 bits here to have globally unique identifiers with little chance
/// of collision, but we use xxhash which for the moment only supports 64 bit.
//
#[ derive( Clone, PartialEq, Eq, Hash ) ]
//
pub(crate) struct UniqueID
{
	bytes: Bytes,
}


impl UniqueID
{
	/// Generate a random ID
	//
	pub(crate) fn random() -> Self
	{
		let mut buf = BytesMut::with_capacity( 16 );
		let mut rng = rand::thread_rng();

		// u128 doesn't work in wasm and serde is being a pain, so 2 u64
		//
		let a = rng.gen::<u64>();
		let b = rng.gen::<u64>();

		let mut wtr = vec![];
		wtr.write_u64::<LittleEndian>( a ).unwrap();
		wtr.write_u64::<LittleEndian>( b ).unwrap();

		buf.extend( wtr );

		Self { bytes: Bytes::from( buf ) }
	}


	/// Seed the UniqueID. It might be data that will be hashed to generate the id.
	/// An identical input here should always give an identical UniqueID.
	//
	pub(crate) fn from_seed( data: &[u8] ) -> Self
	{
		let mut h = XxHash::default();

		for byte in data
		{
			h.write_u8( *byte );
		}

		// The format of the multiservice message requires this to be 128 bits, so add a zero
		// We will have 128bit hash here when xxhash supports 128bit output.
		//
		let mut wtr = vec![];
		wtr.write_u64::<LittleEndian>( h.finish() ).unwrap();
		wtr.write_u64::<LittleEndian>( 0          ).unwrap();

		Self { bytes: Bytes::from( wtr ) }
	}


	/// And empty UniqueID. Can be used to signify the abscence of an id, would usually be all
	/// zero bytes.
	//
	pub(crate) fn null() -> Self
	{
		// The format of the multiservice message requires this to be 128 bits, so add a zero
		// We will have 128bit hash here when xxhash supports 128bit output.
		//
		let mut wtr = vec![];
		wtr.write_u64::<LittleEndian>( 0 ).unwrap();
		wtr.write_u64::<LittleEndian>( 0 ).unwrap();

		Self { bytes: Bytes::from( wtr ) }
	}


	/// Predicate for null values (all bytes are 0).
	//
	pub(crate) fn is_null( &self ) -> bool
	{
		self.bytes.iter().all( |b| *b == 0 )
	}
}



/// Internally is also represented as Bytes, so you just get a copy.
//
impl Into< Bytes > for UniqueID
{
	fn into( self ) -> Bytes
	{
		self.bytes
	}
}


/// The object will just keep the bytes as internal representation, no copies will be made
//
impl TryFrom< Bytes > for UniqueID
{
	type Error = ThesRemoteErr;

	fn try_from( bytes: Bytes ) -> Result<Self, ThesRemoteErr>
	{
		Ok( Self { bytes } )
	}
}


impl fmt::Display for UniqueID
{
	fn fmt( &self, f: &mut fmt::Formatter<'_> ) -> fmt::Result
	{
		write!( f, "{:?}", self )
	}
}



impl fmt::Debug for UniqueID
{
	fn fmt( &self, f: &mut fmt::Formatter<'_> ) -> fmt::Result
	{
		for byte in &self.bytes
		{
			write!( f, "{:02x}", byte )?
		}

		Ok(())
	}
}


#[cfg(test)]
//
mod tests
{
	// What's tested:
	// 1. Identical input data should give identical hash.
	//
	use super::{ *, assert_eq };


	#[test]
	//
	fn identical()
	{
		let sid  = UniqueID::from_seed( b"hi from seed" );
		let sid2 = UniqueID::from_seed( b"hi from seed" );

		assert_eq!( sid, sid2 );
	}


	#[test]
	//
	fn debug()
	{
		let bytes: Bytes = vec![ 0x0, 0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9, 0xA, 0xB, 0xC, 0xD, 0xE, 0xF, ].into();
		let sid = UniqueID::try_from( bytes ).unwrap();

		assert_eq!( "000102030405060708090a0b0c0d0e0f", &format!( "{:?}", sid ) );
	}
}

