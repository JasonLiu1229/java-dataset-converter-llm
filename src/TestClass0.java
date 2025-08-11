
public class TestClass0 {

    @Test
    public void testSingleChunkUnencrypted() throws IOException, Exception {
        byte[] message = "Single Chunk Message".getBytes();
        HCSCore hcsCore = null;
        hcsCore = new HCSCore().builder("0", "./src/test/resources/config.yaml", "./src/test/resources/dotenv.test").withMessageSigningKey(Ed25519PrivateKey.generate());
        hcsCore.addOrUpdateAppParticipant("1", "302a300506032b6570032100c969fbb7b67b36f5560aa59a754a38bd88fd53ff870dad33011bbe2f37f34396", "817c2d3fc1188a7007bce96d5760dd06d3635f378322c98085b4bb37d63c2449");
        ApplicationMessage userMessageToApplicationMessage = OutboundHCSMessage.userMessageToApplicationMessage(new TransactionId(new AccountId(1234L)), message, null, null);
        List<ApplicationMessageChunk> chunks = OutboundHCSMessage.chunk(userMessageToApplicationMessage);
        assertTrue(chunks.size() == 1);
        ConsensusTopicId consensusTopicId = new ConsensusTopicId(1, 2, 3);
        com.hedera.hashgraph.proto.Timestamp timestamp2 = com.hedera.hashgraph.proto.Timestamp.newBuilder().setSeconds(Instant.now().getEpochSecond()).setNanos(Instant.now().getNano()).build();
        ConsensusTopicResponse consensusTopicResponse = ConsensusTopicResponse.newBuilder().setConsensusTimestamp(timestamp2).setMessage(ByteString.copyFromUtf8("message")).setRunningHash(ByteString.copyFromUtf8("runninghash")).setSequenceNumber(20).build();
        SxcConsensusMessage sxcConsensusMessage = new SxcConsensusMessage(consensusTopicId, consensusTopicResponse);
        OnHCSMessageCallback cb = new OnHCSMessageCallback(hcsCore);
        assertDoesNotThrow(() -> {
            cb.partialMessage(chunks.get(0), sxcConsensusMessage);
        });
    }
}
