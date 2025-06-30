#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::{
        consensus::*, producer::*, types::*, phases::*,
    };
    use catalyst_utils::logging::{LogConfig, init_logger};
    use catalyst_utils::metrics::init_metrics;
    use tokio_test;
    use std::time::Duration;

    async fn setup_test_environment() {
        // Initialize logging for tests
        let _ = init_logger(LogConfig::default());
        let _ = init_metrics();
    }

    #[tokio::test]
    async fn test_full_consensus_cycle() {
        setup_test_environment().await;

        let config = ConsensusConfig {
            cycle_duration_ms: 10000,
            construction_phase_ms: 2000,
            campaigning_phase_ms: 2000,
            voting_phase_ms: 2000,
            synchronization_phase_ms: 2000,
            freeze_window_ms: 500,
            min_producers: 1,
            confidence_threshold: 0.6,
        };

        // Create test producers
        let producer1 = Producer::new(
            "producer_1".to_string(),
            [1u8; 32],
            1,
        );

        let producer2 = Producer::new(
            "producer_2".to_string(),
            [2u8; 32],
            1,
        );

        let manager1 = ProducerManager::new(producer1, config.clone());
        let manager2 = ProducerManager::new(producer2, config.clone());

        // Create test transactions
        let transactions = vec![
            TransactionEntry {
                public_key: [10u8; 32],
                amount: -100,  // Spending
                signature: [20u8; 64],
            },
            TransactionEntry {
                public_key: [11u8; 32],
                amount: 100,   // Receiving
                signature: [21u8; 64],
            },
        ];

        // Test construction phase
        let quantity1 = manager1.execute_construction_phase(transactions.clone()).await;
        let quantity2 = manager2.execute_construction_phase(transactions).await;

        assert!(quantity1.is_ok());
        assert!(quantity2.is_ok());

        let q1 = quantity1.unwrap();
        let q2 = quantity2.unwrap();

        // Both producers should generate the same hash for the same transactions
        assert_eq!(q1.first_hash, q2.first_hash);
        assert_eq!(q1.producer_id, "producer_1");
        assert_eq!(q2.producer_id, "producer_2");
    }

    #[tokio::test]
    async fn test_construction_phase_isolation() {
        setup_test_environment().await;

        let producer = Producer::new(
            "test_producer".to_string(),
            [0u8; 32],
            1,
        );

        let mut construction = ConstructionPhase::new(producer);

        let transactions = vec![
            TransactionEntry {
                public_key: [1u8; 32],
                amount: -50,
                signature: [10u8; 64],
            },
            TransactionEntry {
                public_key: [2u8; 32],
                amount: 50,
                signature: [11u8; 64],
            },
        ];

        let result = construction.execute(transactions).await;
        assert!(result.is_ok());

        let quantity = result.unwrap();
        assert_eq!(quantity.producer_id, "test_producer");
        assert!(quantity.first_hash != [0u8; 32]); // Should have a valid hash
    }

    #[tokio::test]
    async fn test_campaigning_phase_majority_detection() {
        setup_test_environment().await;

        let producer = Producer::new(
            "test_producer".to_string(),
            [0u8; 32],
            1,
        );

        let mut campaigning = CampaigningPhase::new(producer);

        // Add multiple quantities with same hash (majority)
        let common_hash = [1u8; 32];
        let minority_hash = [2u8; 32];

        for i in 0..3 {
            campaigning.add_quantity(ProducerQuantity {
                first_hash: common_hash,
                producer_id: format!("producer_{}", i),
                timestamp: current_timestamp_ms(),
            });
        }

        // Add one minority vote
        campaigning.add_quantity(ProducerQuantity {
            first_hash: minority_hash,
            producer_id: "minority_producer".to_string(),
            timestamp: current_timestamp_ms(),
        });

        let result = campaigning.execute(2, 0.6).await;
        assert!(result.is_ok());

        let candidate = result.unwrap();
        assert_eq!(candidate.majority_hash, common_hash);
    }

    #[tokio::test]
    async fn test_voting_phase_compensation_entries() {
        setup_test_environment().await;

        let mut producer = Producer::new(
            "test_producer".to_string(),
            [0u8; 32],
            1,
        );

        // Set up producer state as if construction and campaigning completed
        producer.first_hash = Some([1u8; 32]);
        producer.partial_update = Some(PartialLedgerStateUpdate {
            transaction_entries: vec![],
            transaction_signatures_hash: [0u8; 32],
            total_fees: 0,
            timestamp: current_timestamp_ms(),
        });

        let mut voting = VotingPhase::new(producer);

        // Add candidates with same majority hash
        for i in 0..3 {
            voting.add_candidate(ProducerCandidate {
                majority_hash: [1u8; 32],
                producer_list_hash: [10u8; 32],
                producer_id: format!("producer_{}", i),
                timestamp: current_timestamp_ms(),
            });
        }

        let reward_config = RewardConfig {
            producer_reward: 1000,
            voter_reward: 100,
            total_new_tokens: 10000,
        };

        let result = voting.execute(2, 0.6, &reward_config).await;
        assert!(result.is_ok());

        let vote = result.unwrap();
        assert_eq!(vote.producer_id, "test_producer");
    }

    #[tokio::test]
    async fn test_synchronization_phase_finalization() {
        setup_test_environment().await;

        let mut producer = Producer::new(
            "test_producer".to_string(),
            [0u8; 32],
            1,
        );

        // Set up complete producer state
        let ledger_update = LedgerStateUpdate {
            partial_update: PartialLedgerStateUpdate {
                transaction_entries: vec![],
                transaction_signatures_hash: [0u8; 32],
                total_fees: 0,
                timestamp: current_timestamp_ms(),
            },
            compensation_entries: vec![],
            cycle_number: 1,
            producer_list: vec!["test_producer".to_string()],
            vote_list: vec!["test_producer".to_string()],
        };

        producer.ledger_update = Some(ledger_update);

        let mut synchronization = SynchronizationPhase::new(producer);

        // Add votes with same ledger state hash
        let ledger_hash = hash_data(&synchronization.producer.ledger_update.as_ref().unwrap()).unwrap();

        for i in 0..3 {
            synchronization.add_vote(ProducerVote {
                ledger_state_hash: ledger_hash,
                vote_list_hash: [20u8; 32],
                producer_id: format!("producer_{}", i),
                timestamp: current_timestamp_ms(),
            });
        }

        let result = synchronization.execute(2, 0.6).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert_eq!(output.producer_id, "test_producer");
        assert!(output.dfs_address != [0u8; 21]); // Should have a valid address
    }

    #[tokio::test]
    async fn test_producer_selection_deterministic() {
        let worker_pool = vec![
            "alice".to_string(),
            "bob".to_string(),
            "charlie".to_string(),
            "diana".to_string(),
            "eve".to_string(),
        ];

        let previous_hash = [42u8; 32];
        
        // Multiple calls with same parameters should yield same result
        let selection1 = ProducerSelection::select_producers(
            100,
            previous_hash,
            &worker_pool,
            3,
        ).unwrap();

        let selection2 = ProducerSelection::select_producers(
            100,
            previous_hash,
            &worker_pool,
            3,
        ).unwrap();

        assert_eq!(selection1, selection2);
        assert_eq!(selection1.len(), 3);

        // Different cycle should yield different result
        let selection3 = ProducerSelection::select_producers(
            101,
            previous_hash,
            &worker_pool,
            3,
        ).unwrap();

        assert_ne!(selection1, selection3);
    }

    #[tokio::test]
    async fn test_message_serialization() {
        // Test ProducerQuantity serialization
        let quantity = ProducerQuantity {
            first_hash: [42u8; 32],
            producer_id: "test_producer".to_string(),
            timestamp: current_timestamp_ms(),
        };

        let serialized = quantity.serialize().unwrap();
        let deserialized = ProducerQuantity::deserialize(&serialized).unwrap();
        assert_eq!(quantity, deserialized);

        // Test ProducerCandidate serialization
        let candidate = ProducerCandidate {
            majority_hash: [43u8; 32],
            producer_list_hash: [44u8; 32],
            producer_id: "test_producer".to_string(),
            timestamp: current_timestamp_ms(),
        };

        let serialized = candidate.serialize().unwrap();
        let deserialized = ProducerCandidate::deserialize(&serialized).unwrap();
        assert_eq!(candidate, deserialized);
    }

    #[tokio::test]
    async fn test_consensus_config_validation() {
        let config = ConsensusConfig::default();
        
        // Validate total time makes sense
        let total_phase_time = config.construction_phase_ms + 
                              config.campaigning_phase_ms + 
                              config.voting_phase_ms + 
                              config.synchronization_phase_ms;
        
        assert!(total_phase_time <= config.cycle_duration_ms);
        assert!(config.confidence_threshold > 0.5);
        assert!(config.confidence_threshold <= 1.0);
        assert!(config.min_producers > 0);
    }

    #[tokio::test]
    async fn test_error_handling() {
        setup_test_environment().await;

        let producer = Producer::new(
            "test_producer".to_string(),
            [0u8; 32],
            1,
        );

        let mut campaigning = CampaigningPhase::new(producer);

        // Test insufficient data error
        let result = campaigning.execute(5, 0.75).await; // Require 5, but have 0
        assert!(result.is_err());
        
        match result.unwrap_err().downcast_ref::<ConsensusError>() {
            Some(ConsensusError::InsufficientData { got, required }) => {
                assert_eq!(*got, 0);
                assert_eq!(*required, 5);
            }
            _ => panic!("Expected InsufficientData error"),
        }
    }
}