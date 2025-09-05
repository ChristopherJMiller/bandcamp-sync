//! CD import and ripping functionality

pub mod models;
pub mod musicbrainz;
pub mod reader;
pub mod ripper;

pub use models::CDAlbum;
pub use musicbrainz::MusicBrainzClient;
pub use reader::CDReader;
pub use ripper::CDRipper;
