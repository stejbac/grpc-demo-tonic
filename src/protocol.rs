use musig2::{AggNonce, KeyAggContext, LiftedSignature, NonceSeed, PartialSignature, PubNonce,
    SecNonce, SecNonceBuilder};
use secp::{Point, MaybeScalar, Scalar};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::prelude::rust_2021::*;
use std::sync::{Arc, LazyLock, Mutex};
use thiserror::Error;

pub trait TradeModelStore {
    fn add_trade_model(&self, trade_state: TradeModel);
    fn get_trade_model(&self, trade_id: &str) -> Option<Arc<Mutex<TradeModel>>>;
}

type TradeModelMemoryStore = Mutex<BTreeMap<String, Arc<Mutex<TradeModel>>>>;

impl TradeModelStore for TradeModelMemoryStore {
    fn add_trade_model(&self, trade_model: TradeModel) {
        // TODO: Maybe use try_insert (or similar), to disallow overwriting a trade model with the same ID.
        self.lock().unwrap().insert(trade_model.trade_id.clone(), Arc::new(Mutex::new(trade_model)));
    }

    fn get_trade_model(&self, trade_id: &str) -> Option<Arc<Mutex<TradeModel>>> {
        self.lock().unwrap().get(trade_id).map(Arc::clone)
    }
}

pub static TRADE_MODELS: LazyLock<TradeModelMemoryStore> = LazyLock::new(|| Mutex::new(BTreeMap::new()));

#[derive(Default)]
pub struct TradeModel {
    trade_id: String,
    my_role: Role,
    pub trade_amount: Option<u64>,
    pub buyers_security_deposit: Option<u64>,
    pub sellers_security_deposit: Option<u64>,
    pub deposit_tx_fee_rate: Option<f64>,
    pub prepared_tx_fee_rate: Option<f64>,
    buyer_output_key_ctx: KeyCtx,
    seller_output_key_ctx: KeyCtx,
    swap_tx_input_adaptor: Option<Adaptor>,
    swap_tx_input_sig_ctx: SigCtx,
    buyers_warning_tx_buyer_input_sig_ctx: SigCtx,
    buyers_warning_tx_seller_input_sig_ctx: SigCtx,
    sellers_warning_tx_buyer_input_sig_ctx: SigCtx,
    sellers_warning_tx_seller_input_sig_ctx: SigCtx,
    buyers_redirect_tx_input_sig_ctx: SigCtx,
    sellers_redirect_tx_input_sig_ctx: SigCtx,
}

#[derive(Default, Eq, PartialEq)]
pub enum Role {
    #[default] SellerAsMaker,
    SellerAsTaker,
    BuyerAsMaker,
    BuyerAsTaker,
}

pub struct TxInputParamVector<T> {
    pub swap_tx_input_param: T,
    pub buyers_warning_tx_buyer_input_param: T,
    pub buyers_warning_tx_seller_input_param: T,
    pub sellers_warning_tx_buyer_input_param: T,
    pub sellers_warning_tx_seller_input_param: T,
    pub buyers_redirect_tx_input_param: T,
    pub sellers_redirect_tx_input_param: T,
}

pub struct ExchangedSigs<'a, S: Storage> {
    pub peers_warning_tx_buyer_input_partial_signature: S::Store<'a, PartialSignature>,
    pub peers_warning_tx_seller_input_partial_signature: S::Store<'a, PartialSignature>,
    pub peers_redirect_tx_input_partial_signature: S::Store<'a, PartialSignature>,
    pub swap_tx_input_scalar: Option<SwapTxInputScalar<'a, S>>,
}

pub enum SwapTxInputScalar<'a, S: Storage> {
    BuyersPartialSignature(S::Store<'a, PartialSignature>),
    SellersAdaptor(S::Store<'a, Adaptor>),
}

pub type Adaptor = MaybeScalar;

pub trait Storage {
    type Store<'a, T: 'a>;
}

pub struct ByRef(Infallible);

pub struct ByVal(Infallible);

impl Storage for ByRef {
    type Store<'a, T: 'a> = &'a T;
}

impl Storage for ByVal {
    type Store<'a, T: 'a> = T;
}

pub struct KeyPair {
    pub pub_key: Point,
    pub prv_key: Scalar,
}

pub struct NoncePair {
    pub pub_nonce: PubNonce,
    pub sec_nonce: Option<SecNonce>,
}

#[derive(Default)]
struct KeyCtx {
    am_buyer: bool,
    my_key_share: Option<KeyPair>,
    peers_key_share: Option<Point>,
    aggregated_pub_key: Option<Point>,
    key_agg_ctx: Option<KeyAggContext>,
}

// TODO: For safety, this should hold a reference to the KeyCtx our nonce & signature share (& final
//  aggregation) are built from, so that we don't have to pass it repeatedly as a method parameter.
#[derive(Default)]
struct SigCtx {
    am_buyer: bool,
    my_nonce_share: Option<NoncePair>,
    peers_nonce_share: Option<PubNonce>,
    aggregated_nonce: Option<AggNonce>,
    message: Option<Vec<u8>>,
    my_partial_sig: Option<PartialSignature>,
    peers_partial_sig: Option<PartialSignature>,
    aggregated_sig: Option<LiftedSignature>,
}

impl TradeModel {
    pub fn new(trade_id: String, my_role: Role) -> TradeModel {
        let mut trade_model = TradeModel { trade_id, my_role, ..Default::default() };
        let am_buyer = trade_model.am_buyer();
        trade_model.buyer_output_key_ctx.am_buyer = am_buyer;
        trade_model.seller_output_key_ctx.am_buyer = am_buyer;
        trade_model.swap_tx_input_sig_ctx.am_buyer = am_buyer;
        trade_model.buyers_warning_tx_buyer_input_sig_ctx.am_buyer = am_buyer;
        trade_model.buyers_warning_tx_seller_input_sig_ctx.am_buyer = am_buyer;
        trade_model.sellers_warning_tx_buyer_input_sig_ctx.am_buyer = am_buyer;
        trade_model.sellers_warning_tx_seller_input_sig_ctx.am_buyer = am_buyer;
        trade_model.buyers_redirect_tx_input_sig_ctx.am_buyer = am_buyer;
        trade_model.sellers_redirect_tx_input_sig_ctx.am_buyer = am_buyer;
        trade_model
    }

    fn am_buyer(&self) -> bool {
        self.my_role == Role::BuyerAsMaker || self.my_role == Role::BuyerAsTaker
    }

    // fn am_taker(&self) -> bool {
    //     self.my_role == Role::BuyerAsTaker || self.my_role == Role::SellerAsTaker
    // }

    pub fn init_my_key_shares(&mut self) {
        self.buyer_output_key_ctx.init_my_key_share();
        self.seller_output_key_ctx.init_my_key_share();
    }

    pub fn get_my_key_shares(&self) -> Option<[&KeyPair; 2]> {
        Some([
            self.buyer_output_key_ctx.my_key_share.as_ref()?,
            self.seller_output_key_ctx.my_key_share.as_ref()?
        ])
    }

    pub fn set_peer_key_shares(&mut self, buyer_output_pub_key: Point, seller_output_pub_key: Point) {
        self.buyer_output_key_ctx.peers_key_share = Some(buyer_output_pub_key);
        self.seller_output_key_ctx.peers_key_share = Some(seller_output_pub_key);
    }

    pub fn aggregate_key_shares(&mut self) -> Result<()> {
        self.buyer_output_key_ctx.aggregate_key_shares()?;
        self.seller_output_key_ctx.aggregate_key_shares()?;
        Ok(())
    }

    pub fn init_my_nonce_shares(&mut self) -> Result<()> {
        for ctx in [
            &mut self.buyers_warning_tx_buyer_input_sig_ctx,
            &mut self.sellers_warning_tx_buyer_input_sig_ctx,
            &mut self.buyers_redirect_tx_input_sig_ctx
        ] {
            ctx.init_my_nonce_share(&self.buyer_output_key_ctx)?;
        }
        for ctx in [
            &mut self.swap_tx_input_sig_ctx,
            &mut self.buyers_warning_tx_seller_input_sig_ctx,
            &mut self.sellers_warning_tx_seller_input_sig_ctx,
            &mut self.sellers_redirect_tx_input_sig_ctx
        ] {
            ctx.init_my_nonce_share(&self.seller_output_key_ctx)?;
        }
        Ok(())
    }

    pub fn get_my_nonce_shares(&self) -> Option<TxInputParamVector<&PubNonce>> {
        Some(TxInputParamVector {
            swap_tx_input_param: &(self.swap_tx_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            buyers_warning_tx_buyer_input_param: &(self.buyers_warning_tx_buyer_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            buyers_warning_tx_seller_input_param: &(self.buyers_warning_tx_seller_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            sellers_warning_tx_buyer_input_param: &(self.sellers_warning_tx_buyer_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            sellers_warning_tx_seller_input_param: &(self.sellers_warning_tx_seller_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            buyers_redirect_tx_input_param: &(self.buyers_redirect_tx_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
            sellers_redirect_tx_input_param: &(self.sellers_redirect_tx_input_sig_ctx.my_nonce_share.as_ref()?.pub_nonce),
        })
    }

    pub fn set_peer_nonce_shares(&mut self, peer_nonce_shares: TxInputParamVector<PubNonce>) {
        self.swap_tx_input_sig_ctx.peers_nonce_share = Some(peer_nonce_shares.swap_tx_input_param);
        self.buyers_warning_tx_buyer_input_sig_ctx.peers_nonce_share = Some(peer_nonce_shares.buyers_warning_tx_buyer_input_param);
        self.buyers_warning_tx_seller_input_sig_ctx.peers_nonce_share = Some(peer_nonce_shares.buyers_warning_tx_seller_input_param);
        self.sellers_warning_tx_buyer_input_sig_ctx.peers_nonce_share = Some(peer_nonce_shares.sellers_warning_tx_buyer_input_param);
        self.sellers_warning_tx_seller_input_sig_ctx.peers_nonce_share = Some(peer_nonce_shares.sellers_warning_tx_seller_input_param);
        self.buyers_redirect_tx_input_sig_ctx.peers_nonce_share = Some(peer_nonce_shares.buyers_redirect_tx_input_param);
        self.sellers_redirect_tx_input_sig_ctx.peers_nonce_share = Some(peer_nonce_shares.sellers_redirect_tx_input_param);
    }

    pub fn aggregate_nonce_shares(&mut self) -> Result<()> {
        self.swap_tx_input_sig_ctx.aggregate_nonce_shares()?;
        self.buyers_warning_tx_buyer_input_sig_ctx.aggregate_nonce_shares()?;
        self.buyers_warning_tx_seller_input_sig_ctx.aggregate_nonce_shares()?;
        self.sellers_warning_tx_buyer_input_sig_ctx.aggregate_nonce_shares()?;
        self.sellers_warning_tx_seller_input_sig_ctx.aggregate_nonce_shares()?;
        self.buyers_redirect_tx_input_sig_ctx.aggregate_nonce_shares()?;
        self.sellers_redirect_tx_input_sig_ctx.aggregate_nonce_shares()?;
        Ok(())
    }

    pub fn sign_partial(&mut self) -> Result<()> {
        // TODO: Make these dummy messages (txs-to-sign) non-fixed, for greater realism:
        let [buyer_key_ctx, seller_key_ctx] = [&self.buyer_output_key_ctx, &self.seller_output_key_ctx];

        self.buyers_warning_tx_buyer_input_sig_ctx
            .sign_partial(buyer_key_ctx, b"buyer's warning tx buyer input".into())?;
        self.sellers_warning_tx_buyer_input_sig_ctx
            .sign_partial(buyer_key_ctx, b"seller's warning tx buyer input".into())?;
        self.buyers_redirect_tx_input_sig_ctx
            .sign_partial(buyer_key_ctx, b"buyer's redirect tx input".into())?;

        self.swap_tx_input_sig_ctx
            .sign_partial(seller_key_ctx, b"swap tx input".into())?;
        self.buyers_warning_tx_seller_input_sig_ctx
            .sign_partial(seller_key_ctx, b"buyer's warning tx seller input".into())?;
        self.sellers_warning_tx_seller_input_sig_ctx
            .sign_partial(seller_key_ctx, b"seller's warning tx seller input".into())?;
        self.sellers_redirect_tx_input_sig_ctx
            .sign_partial(seller_key_ctx, b"seller's redirect tx input".into())?;

        if !self.am_buyer() {
            // TODO: Make sure that this simplistic difference, between the seller's multisig private key
            //  share and the seller's signature share on the swap tx, actually gives a secure adaptor.
            //  !! NEEDS CAREFUL CONSIDERATION !! (There may be a better / more standard approach.)
            let sellers_prv_key_share = seller_key_ctx.my_key_share.as_ref().unwrap().prv_key;
            let sellers_sig_share = *self.swap_tx_input_sig_ctx.my_partial_sig.as_ref().unwrap();
            self.swap_tx_input_adaptor = Some(sellers_prv_key_share - sellers_sig_share);
        }
        Ok(())
    }

    pub fn get_my_partial_signatures_on_peer_txs(&self) -> Option<ExchangedSigs<ByRef>> {
        Some(if self.am_buyer() {
            ExchangedSigs {
                peers_warning_tx_buyer_input_partial_signature: self.sellers_warning_tx_buyer_input_sig_ctx.my_partial_sig.as_ref()?,
                peers_warning_tx_seller_input_partial_signature: self.sellers_warning_tx_seller_input_sig_ctx.my_partial_sig.as_ref()?,
                peers_redirect_tx_input_partial_signature: self.sellers_redirect_tx_input_sig_ctx.my_partial_sig.as_ref()?,
                swap_tx_input_scalar: Some(SwapTxInputScalar::BuyersPartialSignature(self.swap_tx_input_sig_ctx.my_partial_sig.as_ref()?)),
            }
        } else {
            ExchangedSigs {
                peers_warning_tx_buyer_input_partial_signature: self.buyers_warning_tx_buyer_input_sig_ctx.my_partial_sig.as_ref()?,
                peers_warning_tx_seller_input_partial_signature: self.buyers_warning_tx_seller_input_sig_ctx.my_partial_sig.as_ref()?,
                peers_redirect_tx_input_partial_signature: self.buyers_redirect_tx_input_sig_ctx.my_partial_sig.as_ref()?,
                swap_tx_input_scalar: Some(SwapTxInputScalar::SellersAdaptor(self.swap_tx_input_adaptor.as_ref()?)),
            }
        })
    }

    pub fn set_peer_partial_signatures_on_my_txs(&mut self, sigs: ExchangedSigs<ByVal>) {
        if self.am_buyer() {
            self.buyers_warning_tx_buyer_input_sig_ctx.peers_partial_sig = Some(sigs.peers_warning_tx_buyer_input_partial_signature);
            self.buyers_warning_tx_seller_input_sig_ctx.peers_partial_sig = Some(sigs.peers_warning_tx_seller_input_partial_signature);
            self.buyers_redirect_tx_input_sig_ctx.peers_partial_sig = Some(sigs.peers_redirect_tx_input_partial_signature);
            if let Some(SwapTxInputScalar::SellersAdaptor(adaptor)) = sigs.swap_tx_input_scalar {
                self.swap_tx_input_adaptor = Some(adaptor); // TODO: The buyer MUST check that the adaptor is valid!
            }
        } else {
            self.sellers_warning_tx_buyer_input_sig_ctx.peers_partial_sig = Some(sigs.peers_warning_tx_buyer_input_partial_signature);
            self.sellers_warning_tx_seller_input_sig_ctx.peers_partial_sig = Some(sigs.peers_warning_tx_seller_input_partial_signature);
            self.sellers_redirect_tx_input_sig_ctx.peers_partial_sig = Some(sigs.peers_redirect_tx_input_partial_signature);
            if let Some(SwapTxInputScalar::BuyersPartialSignature(sig)) = sigs.swap_tx_input_scalar {
                // NOTE: This branch would not normally be used. The buyer should redact this field at the trade start
                // and reveal it later, after payment is started, to prevent premature trade closure by the seller.
                self.swap_tx_input_sig_ctx.peers_partial_sig = Some(sig);
            }
        }
    }

    pub fn aggregate_partial_signatures(&mut self) -> Result<()> {
        if self.am_buyer() {
            self.buyers_warning_tx_buyer_input_sig_ctx.aggregate_partial_signatures(&self.buyer_output_key_ctx)?;
            self.buyers_warning_tx_seller_input_sig_ctx.aggregate_partial_signatures(&self.seller_output_key_ctx)?;
            self.buyers_redirect_tx_input_sig_ctx.aggregate_partial_signatures(&self.buyer_output_key_ctx)?;
        } else {
            self.sellers_warning_tx_buyer_input_sig_ctx.aggregate_partial_signatures(&self.buyer_output_key_ctx)?;
            self.sellers_warning_tx_seller_input_sig_ctx.aggregate_partial_signatures(&self.seller_output_key_ctx)?;
            self.sellers_redirect_tx_input_sig_ctx.aggregate_partial_signatures(&self.seller_output_key_ctx)?;
        }
        Ok(())
    }

    pub fn set_swap_tx_input_peers_partial_signature(&mut self, sig: PartialSignature) -> Result<()> {
        self.swap_tx_input_sig_ctx.peers_partial_sig = Some(sig);
        // TODO: If buyer, apply adaptor to determine the seller's private key share on the buyer output.
        Ok(())
    }

    pub fn aggregate_swap_tx_partial_signatures(&mut self) -> Result<()> {
        let my_key_ctx = if self.am_buyer() { &self.buyer_output_key_ctx } else { &self.seller_output_key_ctx };
        self.swap_tx_input_sig_ctx.aggregate_partial_signatures(my_key_ctx)?;
        Ok(())
    }

    pub fn get_my_private_key_share_for_peer_output(&self) -> Option<&Scalar> {
        // TODO: Check that it's actually safe to release the funds at this point.
        let peer_key_ctx = if self.am_buyer() { &self.seller_output_key_ctx } else { &self.buyer_output_key_ctx };
        Some(&peer_key_ctx.my_key_share.as_ref()?.prv_key)
    }

    pub fn set_peer_private_key_share_for_my_output(&mut self, _prv_key_share: Scalar) -> Result<()> {
        // TODO: Implement.
        Ok(())
    }
}

// impl<T> From<[T; 7]> for TxInputParamVector<T> {
//     fn from(value: [T; 7]) -> Self {
//         let value: (T, T, T, T, T, T, T) = value.into();
//         TxInputParamVector {
//             swap_tx_input_param: value.0,
//             buyers_warning_tx_buyer_input_param: value.1,
//             buyers_warning_tx_seller_input_param: value.2,
//             sellers_warning_tx_buyer_input_param: value.3,
//             sellers_warning_tx_seller_input_param: value.4,
//             buyers_redirect_tx_input_param: value.5,
//             sellers_redirect_tx_input_param: value.6,
//         }
//     }
// }
//
// impl<T> From<TxInputParamVector<T>> for [T; 7] {
//     fn from(value: TxInputParamVector<T>) -> Self {
//         [value.swap_tx_input_param,
//             value.buyers_warning_tx_buyer_input_param, value.buyers_warning_tx_seller_input_param,
//             value.sellers_warning_tx_buyer_input_param, value.sellers_warning_tx_seller_input_param,
//             value.buyers_redirect_tx_input_param, value.sellers_redirect_tx_input_param]
//     }
// }
//
// impl<'a, T> From<&'a TxInputParamVector<T>> for [&'a T; 7] {
//     fn from(value: &'a TxInputParamVector<T>) -> Self {
//         [&value.swap_tx_input_param,
//             &value.buyers_warning_tx_buyer_input_param, &value.buyers_warning_tx_seller_input_param,
//             &value.sellers_warning_tx_buyer_input_param, &value.sellers_warning_tx_seller_input_param,
//             &value.buyers_redirect_tx_input_param, &value.sellers_redirect_tx_input_param]
//     }
// }
//
// impl<'a, T> From<&'a mut TxInputParamVector<T>> for [&'a mut T; 7] {
//     fn from(value: &'a mut TxInputParamVector<T>) -> Self {
//         [&mut value.swap_tx_input_param,
//             &mut value.buyers_warning_tx_buyer_input_param, &mut value.buyers_warning_tx_seller_input_param,
//             &mut value.sellers_warning_tx_buyer_input_param, &mut value.sellers_warning_tx_seller_input_param,
//             &mut value.buyers_redirect_tx_input_param, &mut value.sellers_redirect_tx_input_param]
//     }
// }

impl KeyPair {
    fn new() -> KeyPair {
        KeyPair {
            // pub_key: "029ffbe722b147f3035c87cb1c60b9a5947dd49c774cc31e94773478711a929ac0".parse::<Point>().unwrap(),
            pub_key: Scalar::one().base_point_mul(),
            prv_key: Scalar::one(),
        }
    }
}

impl NoncePair {
    fn new(nonce_seed: impl Into<NonceSeed>, aggregated_pub_key: Point) -> NoncePair {
        let sec_nonce = SecNonceBuilder::new(nonce_seed)
            .with_aggregated_pubkey(aggregated_pub_key)
            .build();
        NoncePair { pub_nonce: sec_nonce.public_nonce(), sec_nonce: Some(sec_nonce) }
    }
}

impl KeyCtx {
    fn init_my_key_share(&mut self) {
        // FIXME: Obtains a dummy private key -- may need to pass a provider or RNG to the constructor.
        self.my_key_share = Some(KeyPair::new());
    }

    fn get_key_shares(&self) -> Option<[Point; 2]> {
        Some(if self.am_buyer {
            [self.my_key_share.as_ref()?.pub_key.clone(), self.peers_key_share.clone()?]
        } else {
            [self.peers_key_share.clone()?, self.my_key_share.as_ref()?.pub_key.clone()]
        })
    }

    fn aggregate_key_shares(&mut self) -> Result<()> {
        let agg_ctx = KeyAggContext::new(self.get_key_shares()
            .ok_or(ProtocolErrorKind::MissingKeyShare)?)?;
        self.aggregated_pub_key = Some(agg_ctx.aggregated_pubkey());
        self.key_agg_ctx = Some(agg_ctx);
        Ok(())
    }
}

impl SigCtx {
    fn init_my_nonce_share(&mut self, key_ctx: &KeyCtx) -> Result<()> {
        // FIXME: Obtains a fixed nonce share -- must pass a _random_ seed data source to the constructor.
        let aggregated_pub_key = key_ctx.aggregated_pub_key.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggPubKey)?.clone();
        self.my_nonce_share = Some(NoncePair::new(&[0; 32], aggregated_pub_key));
        Ok(())
    }

    fn get_nonce_shares(&self) -> Option<[&PubNonce; 2]> {
        Some(if self.am_buyer {
            [&self.my_nonce_share.as_ref()?.pub_nonce, self.peers_nonce_share.as_ref()?]
        } else {
            [self.peers_nonce_share.as_ref()?, &self.my_nonce_share.as_ref()?.pub_nonce]
        })
    }

    fn aggregate_nonce_shares(&mut self) -> Result<()> {
        // TODO: Should check that the aggregated nonce doesn't have a zero point & fail immediately
        //  otherwise. (No need to assign blame at the signing stage, as this is two-party.)
        self.aggregated_nonce = Some(AggNonce::sum(self.get_nonce_shares()
            .ok_or(ProtocolErrorKind::MissingKeyShare)?));
        Ok(())
    }

    fn sign_partial(&mut self, key_ctx: &KeyCtx, message: Vec<u8>) -> Result<&PartialSignature> {
        let key_agg_ctx = key_ctx.key_agg_ctx.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggPubKey)?;
        let seckey = key_ctx.my_key_share.as_ref()
            .ok_or(ProtocolErrorKind::MissingKeyShare)?.prv_key;
        let secnonce = self.my_nonce_share.as_mut()
            .ok_or(ProtocolErrorKind::MissingNonceShare)?.sec_nonce.take()
            .ok_or(ProtocolErrorKind::NonceReuse)?;
        let aggregated_nonce = &self.aggregated_nonce.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggNonce)?;

        let sig = musig2::sign_partial(key_agg_ctx, seckey, secnonce, aggregated_nonce, &message[..])?;
        self.message = Some(message);
        Ok(self.my_partial_sig.insert(sig))
    }

    fn get_partial_signatures(&self) -> Option<[PartialSignature; 2]> {
        Some(if self.am_buyer {
            [self.my_partial_sig.clone()?, self.peers_partial_sig.clone()?]
        } else {
            [self.peers_partial_sig.clone()?, self.my_partial_sig.clone()?]
        })
    }

    fn aggregate_partial_signatures(&mut self, key_ctx: &KeyCtx) -> Result<&LiftedSignature> {
        let key_agg_ctx = key_ctx.key_agg_ctx.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggPubKey)?;
        let aggregated_nonce = &self.aggregated_nonce.as_ref()
            .ok_or(ProtocolErrorKind::MissingAggNonce)?;
        let partial_signatures = self.get_partial_signatures()
            .ok_or(ProtocolErrorKind::MissingPartialSig)?;
        let message = &self.message.as_ref()
            .ok_or(ProtocolErrorKind::MissingPartialSig)?[..];

        let sig = musig2::aggregate_partial_signatures(key_agg_ctx, aggregated_nonce, partial_signatures, message)?;
        Ok(self.aggregated_sig.insert(sig))
    }
}

type Result<T> = std::result::Result<T, ProtocolErrorKind>;

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ProtocolErrorKind {
    #[error("missing key share")]
    MissingKeyShare,
    #[error("missing nonce share")]
    MissingNonceShare,
    #[error("missing partial signature")]
    MissingPartialSig,
    #[error("missing aggregated pubkey")]
    MissingAggPubKey,
    #[error("missing aggregated nonce")]
    MissingAggNonce,
    #[error("nonce has already been used")]
    NonceReuse,
    KeyAgg(#[from] musig2::errors::KeyAggError),
    Signing(#[from] musig2::errors::SigningError),
    Verify(#[from] musig2::errors::VerifyError),
}
