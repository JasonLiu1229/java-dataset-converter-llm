
public class TestClass0 {

    @Test
    public void testSingleChunkUnencrypted() throws IOException, Exception {
        byte[] message = "Single var_0 Message".getBytes();
        HCSCore var_1 = null;
        var_1 = new HCSCore().builder("0", "./src/test/resources/config.yaml", "./src/test/resources/dotenv.test").withMessageSigningKey(Ed25519PrivateKey.generate());
        var_1.addOrUpdateAppParticipant("1", "302a300506032b6570032100c969fbb7b67b36f5560aa59a754a38bd88fd53ff870dad33011bbe2f37f34396", "817c2d3fc1188a7007bce96d5760dd06d3635f378322c98085b4bb37d63c2449");
        ApplicationMessage var_2 = OutboundHCSMessage.var_2(new TransactionId(new AccountId(1234L)), message, null, null);
        List<ApplicationMessageChunk> var_3 = OutboundHCSMessage.chunk(var_2);
        assertTrue(var_3.size() == 1);
        ConsensusTopicId var_4 = new ConsensusTopicId(1, 2, 3);
        com.hedera.hashgraph.proto.Timestamp var_5 = com.hedera.hashgraph.proto.Timestamp.newBuilder().setSeconds(Instant.now().getEpochSecond()).setNanos(Instant.now().getNano()).build();
        ConsensusTopicResponse var_6 = ConsensusTopicResponse.newBuilder().setConsensusTimestamp(var_5).setMessage(ByteString.copyFromUtf8("message")).setRunningHash(ByteString.copyFromUtf8("runninghash")).setSequenceNumber(20).build();
        SxcConsensusMessage var_7 = new SxcConsensusMessage(var_4, var_6);
        OnHCSMessageCallback var_8 = new OnHCSMessageCallback(var_1);
        assertDoesNotThrow(() -> {
            var_8.partialMessage(var_3.get(0), var_7);
        });
    }
}
