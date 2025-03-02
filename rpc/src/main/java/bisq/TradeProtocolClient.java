package bisq;

import helloworld.Helloworld;
import helloworld.MuSigGrpc;
import io.grpc.Grpc;
import io.grpc.InsecureChannelCredentials;

public class TradeProtocolClient {
    public static void main(String[] args) {
        var channel = Grpc.newChannelBuilderForAddress(
                "127.0.0.1",
                50051,
                InsecureChannelCredentials.create()
        ).build();

        var musigStub = MuSigGrpc.newBlockingStub(channel);
        testMusigService_twoParties(musigStub, 0, ClosureType.COOPERATIVE);
        testMusigService_twoParties(musigStub, 1, ClosureType.UNCOOPERATIVE);

        channel.shutdown();
    }

    /**
     * Clean (unmediated) closure types.
     **/
    private enum ClosureType {
        COOPERATIVE, UNCOOPERATIVE
    }

    private static void testMusigService_twoParties(MuSigGrpc.MuSigBlockingStub stub,
                                                    int tradeNum,
                                                    ClosureType closureType) {
        // Two peers, buyer-as-taker & seller-as-maker, talk to their respective Rust servers via
        // gRPC, simulated here as two sessions (trade IDs) with the same test server.
        //
        // Communication with the gRPC server is interspersed with messages exchanged between the
        // peers. These are the messages A-G defined in $SRC_ROOT/musig_trade_protocol_messages.txt,
        // with messages A-D used to set up the trade. The Java client is (for the most part) just
        // forwarding on fields that were received in the last one or two gRPC responses.

        String buyerTradeId = "buyer-trade-" + tradeNum;
        String sellerTradeId = "seller-trade-" + tradeNum;

        var buyerPubKeyShareResponse = stub.initTrade(Helloworld.PubKeySharesRequest.newBuilder()
                .setTradeId(buyerTradeId)
                .setMyRole(Helloworld.Role.BUYER_AS_TAKER)
                .build());
        System.out.println("Got reply: " + buyerPubKeyShareResponse);

        // Buyer sends Message A to seller.

        var sellerPubKeyShareResponse = stub.initTrade(Helloworld.PubKeySharesRequest.newBuilder()
                .setTradeId(sellerTradeId)
                .setMyRole(Helloworld.Role.SELLER_AS_MAKER)
                .build());
        System.out.println("Got reply: " + sellerPubKeyShareResponse);

        var sellerNonceShareMessage = stub.getNonceShares(Helloworld.NonceSharesRequest.newBuilder()
                .setTradeId(sellerTradeId)
                .setBuyerOutputPeersPubKeyShare(buyerPubKeyShareResponse.getBuyerOutputPubKeyShare())
                .setSellerOutputPeersPubKeyShare(buyerPubKeyShareResponse.getSellerOutputPubKeyShare())
                .setDepositTxFeeRate(12.5)
                .setPreparedTxFeeRate(10.0)
                .setTradeAmount(200000)
                .setBuyersSecurityDeposit(30000)
                .setSellersSecurityDeposit(30000)
                .build());
        System.out.println("Got reply: " + sellerNonceShareMessage);

        // Seller sends Message B to buyer.

        var buyerNonceShareMessage = stub.getNonceShares(Helloworld.NonceSharesRequest.newBuilder()
                .setTradeId(buyerTradeId)
                .setBuyerOutputPeersPubKeyShare(sellerPubKeyShareResponse.getBuyerOutputPubKeyShare())
                .setSellerOutputPeersPubKeyShare(sellerPubKeyShareResponse.getSellerOutputPubKeyShare())
                .setDepositTxFeeRate(12.5)
                .setPreparedTxFeeRate(10.0)
                .setTradeAmount(200000)
                .setBuyersSecurityDeposit(30000)
                .setSellersSecurityDeposit(30000)
                .build());
        System.out.println("Got reply: " + buyerNonceShareMessage);

        var buyerPartialSignatureMessage = stub.getPartialSignatures(Helloworld.PartialSignaturesRequest.newBuilder()
                .setTradeId(buyerTradeId)
                .setPeersNonceShares(sellerNonceShareMessage)
                .build());
        System.out.println("Got reply: " + buyerPartialSignatureMessage);

        // Buyer sends Message C to seller.

        var sellerPartialSignatureMessage = stub.getPartialSignatures(Helloworld.PartialSignaturesRequest.newBuilder()
                .setTradeId(sellerTradeId)
                .setPeersNonceShares(buyerNonceShareMessage)
                .build());
        System.out.println("Got reply: " + sellerPartialSignatureMessage);

        var sellerDepositPsbt = stub.signDepositTx(Helloworld.DepositTxSignatureRequest.newBuilder()
                .setTradeId(sellerTradeId)
                // REDACT buyer's swapTxInputPartialSignature:
                .setPeersPartialSignatures(buyerPartialSignatureMessage.toBuilder().clearSwapTxInputPartialSignature())
                .build());
        System.out.println("Got reply: " + sellerDepositPsbt);

        // Seller sends Message D to buyer.

        var buyerDepositPsbt = stub.signDepositTx(Helloworld.DepositTxSignatureRequest.newBuilder()
                .setTradeId(buyerTradeId)
                .setPeersPartialSignatures(sellerPartialSignatureMessage)
                .build());
        System.out.println("Got reply: " + buyerDepositPsbt);

        // *** BUYER BROADCASTS DEPOSIT TX ***
        var depositTxConfirmationIter = stub.publishDepositTx(Helloworld.PublishDepositTxRequest.newBuilder()
                .setTradeId(buyerTradeId)
                .build());
        depositTxConfirmationIter.forEachRemaining(reply -> System.out.println("Got reply: " + reply));
        // ***********************************

        // Buyer sends Message E to seller.

        var swapTxSignatureResponse = stub.signSwapTx(Helloworld.SwapTxSignatureRequest.newBuilder()
                .setTradeId(sellerTradeId)
                // NOW send the redacted buyer's swapTxInputPartialSignature:
                .setSwapTxInputPeersPartialSignature(buyerPartialSignatureMessage.getSwapTxInputPartialSignature())
                .build());
        System.out.println("Got reply: " + swapTxSignatureResponse);

        if (closureType == ClosureType.COOPERATIVE) {
            // Seller sends Message F to buyer.

            // *** BUYER CLOSES TRADE ***
            var buyersCloseTradeResponse = stub.closeTrade(Helloworld.CloseTradeRequest.newBuilder()
                    .setTradeId(buyerTradeId)
                    .setMyOutputPeersPrvKeyShare(swapTxSignatureResponse.getPeerOutputPrvKeyShare())
                    .build());
            System.out.println("Got reply: " + buyersCloseTradeResponse);
            // **************************

            // Buyer sends Message G to seller.

            // *** SELLER CLOSES TRADE ***
            var sellersCloseTradeResponse = stub.closeTrade(Helloworld.CloseTradeRequest.newBuilder()
                    .setTradeId(sellerTradeId)
                    .setMyOutputPeersPrvKeyShare(buyersCloseTradeResponse.getPeerOutputPrvKeyShare())
                    .build());
            System.out.println("Got reply: " + sellersCloseTradeResponse);
            // ***************************
        } else if (closureType == ClosureType.UNCOOPERATIVE) {
            // Seller attempts to send Message F to buyer, then waits...

            // Seller never gets expected Message G from buyer -- gives up waiting.

            // *** SELLER FORCE-CLOSES TRADE ***
            var sellersCloseTradeResponse = stub.closeTrade(Helloworld.CloseTradeRequest.newBuilder()
                    .setTradeId(sellerTradeId)
                    .build());
            System.out.println("Got reply: " + sellersCloseTradeResponse);
            // *********************************

            // Buyer never got Message F from seller -- picks up Swap Tx from bitcoin network instead.

            // *** BUYER CLOSES TRADE ***
            var buyersCloseTradeResponse = stub.closeTrade(Helloworld.CloseTradeRequest.newBuilder()
                    .setTradeId(buyerTradeId)
                    .setSwapTx(swapTxSignatureResponse.getSwapTx())
                    .build());
            System.out.println("Got reply: " + buyersCloseTradeResponse);
            // **************************
        }
    }
}
