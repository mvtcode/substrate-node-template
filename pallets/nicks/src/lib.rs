#![cfg_attr(not(feature = "std"), no_std)]

use sp_std::prelude::*;
use sp_runtime::{
	traits::{StaticLookup, Zero}
};
use frame_support::{
	decl_module, decl_event, decl_storage, ensure, decl_error,
	traits::{Currency, EnsureOrigin, ReservableCurrency, OnUnbalanced, Get},
};
use frame_system::ensure_signed;

type BalanceOf<T> = <<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;
type NegativeImbalanceOf<T> = <<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::NegativeImbalance;

pub trait Config: frame_system::Config {
	/// The overarching event type.
	type Event: From<Event<Self>> + Into<<Self as frame_system::Config>::Event>;

	/// The currency trait.
	type Currency: ReservableCurrency<Self::AccountId>;

	/// Reservation fee.
	type ReservationFee: Get<BalanceOf<Self>>;

	/// What to do with slashed funds.
	type Slashed: OnUnbalanced<NegativeImbalanceOf<Self>>;

	/// The origin which may forcibly set or remove a name. Root can always do this.
	type ForceOrigin: EnsureOrigin<Self::Origin>;

	/// The minimum length a name may be.
	type MinLength: Get<usize>;

	/// The maximum length a name may be.
	type MaxLength: Get<usize>;
}

decl_storage! {
	trait Store for Module<T: Config> as Nicks {
		/// The lookup table for names.
		NameOf: map hasher(twox_64_concat) T::AccountId => Option<(Vec<u8>, BalanceOf<T>)>;
	}
}

decl_event!(
	pub enum Event<T> where AccountId = <T as frame_system::Config>::AccountId, Balance = BalanceOf<T> {
		/// A name was set. \[who\]
		NameSet(AccountId),
		/// A name was forcibly set. \[target\]
		NameForced(AccountId),
		/// A name was changed. \[who\]
		NameChanged(AccountId),
		/// A name was cleared, and the given balance returned. \[who, deposit\]
		NameCleared(AccountId, Balance),
		/// A name was removed and the given balance slashed. \[target, deposit\]
		NameKilled(AccountId, Balance),
	}
);

decl_error! {
	/// Error for the nicks module.
	pub enum Error for Module<T: Config> {
		/// A name is too short.
		TooShort,
		/// A name is too long.
		TooLong,
		/// An account isn't named.
		Unnamed,
	}
}

decl_module! {
	/// Nicks module declaration.
	pub struct Module<T: Config> for enum Call where origin: T::Origin {
		type Error = Error<T>;

		fn deposit_event() = default;

		/// Reservation fee.
		const ReservationFee: BalanceOf<T> = T::ReservationFee::get();

		/// The minimum length a name may be.
		const MinLength: u32 = T::MinLength::get() as u32;

		/// The maximum length a name may be.
		const MaxLength: u32 = T::MaxLength::get() as u32;

		/// Set an account's name. The name should be a UTF-8-encoded string by convention, though
		/// we don't check it.
		///
		/// The name may not be more than `T::MaxLength` bytes, nor less than `T::MinLength` bytes.
		///
		/// If the account doesn't already have a name, then a fee of `ReservationFee` is reserved
		/// in the account.
		///
		/// The dispatch origin for this call must be _Signed_.
		///
		/// # <weight>
		/// - O(1).
		/// - At most one balance operation.
		/// - One storage read/write.
		/// - One event.
		/// # </weight>
		#[weight = 50_000_000]
		fn set_name(origin, name: Vec<u8>) {
			let sender = ensure_signed(origin)?;

			ensure!(name.len() >= T::MinLength::get(), Error::<T>::TooShort);
			ensure!(name.len() <= T::MaxLength::get(), Error::<T>::TooLong);

			let deposit = if let Some((_, deposit)) = <NameOf<T>>::get(&sender) {
				Self::deposit_event(RawEvent::NameChanged(sender.clone()));
				deposit
			} else {
				let deposit = T::ReservationFee::get();
				T::Currency::reserve(&sender, deposit.clone())?;
				Self::deposit_event(RawEvent::NameSet(sender.clone()));
				deposit
			};

			<NameOf<T>>::insert(&sender, (name, deposit));
		}

		/// Clear an account's name and return the deposit. Fails if the account was not named.
		///
		/// The dispatch origin for this call must be _Signed_.
		///
		/// # <weight>
		/// - O(1).
		/// - One balance operation.
		/// - One storage read/write.
		/// - One event.
		/// # </weight>
		#[weight = 70_000_000]
		fn clear_name(origin) {
			let sender = ensure_signed(origin)?;

			let deposit = <NameOf<T>>::take(&sender).ok_or(Error::<T>::Unnamed)?.1;

			let err_amount = T::Currency::unreserve(&sender, deposit.clone());
			debug_assert!(err_amount.is_zero());

			Self::deposit_event(RawEvent::NameCleared(sender, deposit));
		}

		/// Remove an account's name and take charge of the deposit.
		///
		/// Fails if `who` has not been named. The deposit is dealt with through `T::Slashed`
		/// imbalance handler.
		///
		/// The dispatch origin for this call must match `T::ForceOrigin`.
		///
		/// # <weight>
		/// - O(1).
		/// - One unbalanced handler (probably a balance transfer)
		/// - One storage read/write.
		/// - One event.
		/// # </weight>
		#[weight = 70_000_000]
		fn kill_name(origin, target: <T::Lookup as StaticLookup>::Source) {
			T::ForceOrigin::ensure_origin(origin)?;

			// Figure out who we're meant to be clearing.
			let target = T::Lookup::lookup(target)?;
			// Grab their deposit (and check that they have one).
			let deposit = <NameOf<T>>::take(&target).ok_or(Error::<T>::Unnamed)?.1;
			// Slash their deposit from them.
			T::Slashed::on_unbalanced(T::Currency::slash_reserved(&target, deposit.clone()).0);

			Self::deposit_event(RawEvent::NameKilled(target, deposit));
		}

		/// Set a third-party account's name with no deposit.
		///
		/// No length checking is done on the name.
		///
		/// The dispatch origin for this call must match `T::ForceOrigin`.
		///
		/// # <weight>
		/// - O(1).
		/// - At most one balance operation.
		/// - One storage read/write.
		/// - One event.
		/// # </weight>
		#[weight = 70_000_000]
		fn force_name(origin, target: <T::Lookup as StaticLookup>::Source, name: Vec<u8>) {
			T::ForceOrigin::ensure_origin(origin)?;

			let target = T::Lookup::lookup(target)?;
			let deposit = <NameOf<T>>::get(&target).map(|x| x.1).unwrap_or_else(Zero::zero);
			<NameOf<T>>::insert(&target, (name, deposit));

			Self::deposit_event(RawEvent::NameForced(target));
		}
	}
}