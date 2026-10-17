DELETE FROM chat_rate_limits
WHERE (scope = 'm' AND window < ?1) OR (scope = 'h' AND window < ?2)
