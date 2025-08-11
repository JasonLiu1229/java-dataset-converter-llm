
public class TestClass0 {

    @Test
    public void func_1() throws IOException, Exception {
        byte[] var_0 = "Single var_1 Message".getBytes();
        HCSCore var_2 = null;
        var_2 = new HCSCore().builder("0", "./src/test/resources/config.yaml", "./src/test/resources/dotenv.test").withMessageSigningKey(Ed25519PrivateKey.generate());
        var_2.addOrUpdateAppParticipant("1", "302a300506032b6570032100c969fbb7b67b36f5560aa59a754a38bd88fd53ff870dad33011bbe2f37f34396", "817c2d3fc1188a7007bce96d5760dd06d3635f378322c98085b4bb37d63c2449");
        ApplicationMessage var_3 = OutboundHCSMessage.userMessageToApplicationMessage(new TransactionId(new AccountId(1234L)), var_0, null, null);
        List<ApplicationMessageChunk> var_4 = OutboundHCSMessage.chunk(var_3);
        assertTrue(var_4.size() == 1);
        ConsensusTopicId var_5 = new ConsensusTopicId(1, 2, 3);
        com.hedera.hashgraph.proto.Timestamp var_6 = com.hedera.hashgraph.proto.Timestamp.newBuilder().setSeconds(Instant.now().getEpochSecond()).setNanos(Instant.now().getNano()).build();
        ConsensusTopicResponse var_7 = ConsensusTopicResponse.newBuilder().setConsensusTimestamp(var_6).setMessage(ByteString.copyFromUtf8("var_0")).setRunningHash(ByteString.copyFromUtf8("runninghash")).setSequenceNumber(20).build();
        SxcConsensusMessage var_8 = new SxcConsensusMessage(var_5, var_7);
        OnHCSMessageCallback var_9 = new OnHCSMessageCallback(var_2);
        assertDoesNotThrow(() -> {
            var_9.partialMessage(var_4.get(0), var_8);
        });
    }
}
