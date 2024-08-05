//! Benchmarks for scanning side metadata for non-zero bits.

use criterion::Criterion;
use mmtk::util::{
    constants::LOG_BITS_IN_WORD,
    metadata::side_metadata::bench::scan_non_zero_bits_in_metadata_bytes, Address,
};
use rand::{seq::IteratorRandom, SeedableRng};
use rand_chacha::ChaCha8Rng;

fn allocate_aligned(size: usize) -> Address {
    let ptr = unsafe {
        std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(size, size).unwrap())
    };
    Address::from_mut_ptr(ptr)
}

const BLOCK_BYTES: usize = 32768usize; // Match an Immix block size.

// Asssume one-bit-per-word metadata (matching VO bits).
const BLOCK_META_BYTES: usize = BLOCK_BYTES >> LOG_BITS_IN_WORD;

/// Set this many distinct bits in the bitmap.
const NUM_OBJECTS: usize = 200;

/// Get a deterministic seeded Rng.
fn get_rng() -> ChaCha8Rng {
    // Create an Rng from a seed and an explicit Rng type.
    // Not secure at all, but completely deterministic and reproducible.
    // The following seed is read from /dev/random
    const SEED64: u64 = 0x4050cb1b5ab26c70;
    ChaCha8Rng::seed_from_u64(SEED64)
}

/// A bitmap, with known location of each bit for assertion.
struct PreparedBitmap {
    start: Address,
    end: Address,
    set_bits: Vec<(Address, u8)>,
}

/// Make a bitmap of the desired size and set bits.
fn make_standard_bitmap() -> PreparedBitmap {
    let start = allocate_aligned(BLOCK_META_BYTES);
    let end = start + BLOCK_META_BYTES;
    let mut rng = get_rng();

    let mut set_bits = (0..(BLOCK_BYTES >> LOG_BITS_IN_WORD))
        .choose_multiple(&mut rng, NUM_OBJECTS)
        .iter()
        .map(|total_bit_offset| {
            let word_offset = total_bit_offset >> LOG_BITS_IN_WORD;
            let bit_offset = total_bit_offset & ((1 << LOG_BITS_IN_WORD) - 1);
            (start + (word_offset << LOG_BITS_IN_WORD), bit_offset as u8)
        })
        .collect::<Vec<_>>();

    set_bits.sort();

    for (addr, bit) in set_bits.iter() {
        let word = unsafe { addr.load::<usize>() };
        let new_word = word | (1 << bit);
        unsafe { addr.store::<usize>(new_word) };
    }

    PreparedBitmap {
        start,
        end,
        set_bits,
    }
}

pub fn bench(c: &mut Criterion) {
    c.bench_function("bscan_block", |b| {
        let bitmap = make_standard_bitmap();
        let mut holder: Vec<(Address, u8)> = Vec::with_capacity(NUM_OBJECTS);

        b.iter(|| {
            holder.clear();
            scan_non_zero_bits_in_metadata_bytes(bitmap.start, bitmap.end, &mut |addr, shift| {
                holder.push((addr, shift));
            });
        });

        assert_eq!(holder.len(), NUM_OBJECTS);
        assert_eq!(holder, bitmap.set_bits);
    });
}
