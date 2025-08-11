
public class TestClass0 {

    @Test
    public void testSingleChunkUnencrypted() throws IOException, Exception {
        byte[] message = "Single Chunk Message".getBytes();
        HCSCore var_0 = null;
        var_0 = new HCSCore().builder("0", "./src/test/resources/config.yaml", "./src/test/resources/dotenv.test").withMessageSigningKey(Ed25519PrivateKey.generate());
        var_0.addOrUpdateAppParticipant("1", "302a300506032b6570032100c969fbb7b67b36f5560aa59a754a38bd88fd53ff870dad33011bbe2f37f34396", "817c2d3fc1188a7007bce96d5760dd06d3635f378322c98085b4bb37d63c2449");
        ApplicationMessage var_1 = OutboundHCSMessage.var_1(new TransactionId(new AccountId(1234L)), message, null, null);
        List<ApplicationMessageChunk> var_2 = OutboundHCSMessage.chunk(var_1);
        assertTrue(var_2.size() == 1);
        ConsensusTopicId var_3 = new ConsensusTopicId(1, 2, 3);
        com.hedera.hashgraph.proto.Timestamp var_4 = com.hedera.hashgraph.proto.Timestamp.newBuilder().setSeconds(Instant.now().getEpochSecond()).setNanos(Instant.now().getNano()).build();
        ConsensusTopicResponse consensusTopicResponse = ConsensusTopicResponse.newBuilder().setConsensusTimestamp(var_4).setMessage(ByteString.copyFromUtf8("message")).setRunningHash(ByteString.copyFromUtf8("runninghash")).setSequenceNumber(20).build();
        SxcConsensusMessage var_5 = new SxcConsensusMessage(var_3, consensusTopicResponse);
        OnHCSMessageCallback var_6 = new OnHCSMessageCallback(var_0);
        assertDoesNotThrow(() -> {
            var_6.partialMessage(var_2.get(0), var_5);
        });
    }
}
