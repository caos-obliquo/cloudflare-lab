#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# ============================================================
# deploy-localstack.sh
# One-command: start LocalStack → build Lambda → deploy → EventBridge
#
# Requires: LOCALSTACK_AUTH_TOKEN env var (or set below)
#   export LOCALSTACK_AUTH_TOKEN="ls-..."
# ============================================================

# --- Config ---
LAMBDA_NAME="devops-api-dev"
ZIP="target/lambda/devops-api/deploy.zip"
REGION="us-east-1"
EP="http://localhost:4566"
AID="000000000000"
LS_TOKEN="${LOCALSTACK_AUTH_TOKEN:-}"

echo "═══ STEP 1: Podman socket ═══"
pkill -f "podman system service" 2>/dev/null || true
sleep 1
rm -f /tmp/docker.sock
nohup podman system service --time=0 unix:///tmp/docker.sock >/dev/null 2>&1 &
disown
sleep 2
ls -la /tmp/docker.sock

echo ""
echo "═══ STEP 2: Start LocalStack ═══"
podman stop localstack 2>/dev/null || true
podman rm localstack 2>/dev/null || true
podman run --rm -d --name localstack \
  --network bridge \
  -p 4566:4566 \
  -e LOCALSTACK_AUTH_TOKEN="$LS_TOKEN" \
  -e SERVICES=lambda,iam,s3,sts,events \
  -e AWS_DEFAULT_REGION="$REGION" \
  -v /tmp/docker.sock:/var/run/docker.sock \
  docker.io/localstack/localstack:latest

echo "Waiting for LocalStack to be healthy..."
for i in $(seq 1 30); do
    HEALTH=$(curl -4 -s "$EP/_localstack/health" 2>/dev/null || echo "{}")
    if echo "$HEALTH" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('services',{}).get('lambda',''))" 2>/dev/null | grep -q available; then
        echo "LocalStack ready!"
        break
    fi
    sleep 2
done
curl -4 -s "$EP/_localstack/health" | python3 -m json.tool

echo ""
echo "═══ STEP 3: Build Lambda ═══"
cd "$(dirname "$0")/.."
# devops-api is a standalone crate (not in workspace), build from its directory.
cargo build --release --manifest-path lambda/devops-api/Cargo.toml
mkdir -p target/lambda/devops-api
cp target/release/bootstrap target/lambda/devops-api/ 2>/dev/null || true

echo "Creating zip..."
mkdir -p target/lambda/devops-api
zip -j "target/lambda/devops-api/deploy.zip" target/release/bootstrap 2>/dev/null || true
ls -lh "$ZIP"

echo ""
echo "═══ STEP 4: Create IAM role ═══"
ROLE_ARN="arn:aws:iam::$AID:role/lambda-exec-dev"
curl -4 -s -X POST "$EP/" \
    -d "Action=CreateRole&RoleName=lambda-exec-dev&AssumeRolePolicyDocument={\"Version\":\"2012-10-17\",\"Statement\":[{\"Effect\":\"Allow\",\"Principal\":{\"Service\":\"lambda.amazonaws.com\"},\"Action\":\"sts:AssumeRole\"}]}&Version=2010-05-08" >/dev/null
curl -4 -s -X POST "$EP/" \
    -d "Action=AttachRolePolicy&RoleName=lambda-exec-dev&PolicyArn=arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole&Version=2010-05-08" >/dev/null
echo "IAM role ready: $ROLE_ARN"

echo ""
echo "═══ STEP 5: Create Lambda function ═══"
echo "Encoding zip..."
python3 -c "
import base64, json
with open('$ZIP','rb') as f:
    b64 = base64.b64encode(f.read()).decode()
payload = {
    'FunctionName': '$LAMBDA_NAME',
    'Runtime': 'provided.al2023',
    'Role': '$ROLE_ARN',
    'Handler': 'bootstrap',
    'Code': {'ZipFile': b64},
    'Environment': {'Variables': {
        'ENVIRONMENT': 'dev',
        'WORKER_AUTH_URL': 'http://auth.local',
        'WORKER_GATEWAY_URL': 'http://gateway.local',
    }},
}
with open('/tmp/lambda-create.json','w') as f:
    json.dump(payload, f)
print(f'Payload: {len(json.dumps(payload))} bytes')
"

echo "Creating function..."
curl -4 -s -X POST "$EP/2015-03-31/functions" \
    -H "Content-Type: application/json" \
    -d @/tmp/lambda-create.json | python3 -c "
import sys, json
d = json.load(sys.stdin)
c = d.get('Configuration', d)
print(f'State: {c.get(\"State\",\"?\")}')
print(f'ARN: {c.get(\"FunctionArn\",\"?\")}')
"

echo "Waiting for function to become Active..."
for i in $(seq 1 15); do
    STATE=$(curl -4 -s "$EP/2015-03-31/functions/$LAMBDA_NAME" |
        python3 -c "import sys,json; print(json.load(sys.stdin).get('Configuration',{}).get('State',''))" 2>/dev/null)
    if [ "$STATE" = "Active" ]; then
        echo "Lambda Active!"
        break
    fi
    echo "  State: $STATE — waiting..."
    sleep 2
done

echo ""
echo "═══ STEP 6: Create Function URL ═══"
curl -4 -s -X POST "$EP/2021-10-31/functions/$LAMBDA_NAME/url" \
    -H "Content-Type: application/json" \
    -d '{"qualifier":"LATEST","authorization_type":"NONE","cors":{"allow_origins":["*"]}}' >/dev/null
echo "Function URL created (auth: NONE)"

echo ""
echo "═══ STEP 7: Test direct invocation ═══"
echo "Lambda container ID:"
podman ps --filter "name=localstack-lambda" --format "{{.ID}}" | head -1
echo "Invoking..."
curl -4 -s -X POST "$EP/2015-03-31/functions/$LAMBDA_NAME/invocations" \
    -H "Content-Type: application/json" \
    -H "X-Amz-Invocation-Type: RequestResponse" \
    --max-time 60 \
    -d '{"httpMethod":"GET","path":"/health","headers":{},"queryStringParameters":{},"body":null}'

echo ""
echo "═══ STEP 8: EventBridge setup ═══"
echo "Creating rule (JSON API)..."
RULE_ARN=$(curl -4 -s -X POST "$EP/" \
  -H "X-Amz-Target: AWSEvents.PutRule" \
  -H "Content-Type: application/x-amz-json-1.1" \
  -d '{"Name":"catch-all","EventPattern":{"source":[{"prefix":""}]}}' |
  python3 -c "import sys,json; print(json.load(sys.stdin).get('RuleArn',''))" 2>/dev/null)
echo "Rule: $RULE_ARN"

echo "Adding Lambda target..."
curl -4 -s -X POST "$EP/" \
  -H "X-Amz-Target: AWSEvents.PutTargets" \
  -H "Content-Type: application/x-amz-json-1.1" \
  -d '{"Rule":"catch-all","Targets":[{"Id":"1","Arn":"arn:aws:lambda:'"$REGION"':000000000000:function:'"$LAMBDA_NAME"'"}]}' |
  python3 -c "import sys,json; d=json.load(sys.stdin); print(f'Targets added: {d.get(\"FailedEntryCount\",\"?\")} failures')"

echo "Sending test event..."
curl -4 -s -X POST "$EP/" \
  -H "X-Amz-Target: AWSEvents.PutEvents" \
  -H "Content-Type: application/x-amz-json-1.1" \
  -d '{"Entries":[{"Source":"analytics","DetailType":"Test","Detail":"{\"hello\":\"world\"}","EventBusName":"default"}]}' |
  python3 -c "import sys,json; d=json.load(sys.stdin); print(f'Events sent: failed={d.get(\"FailedEntryCount\",\"?\")}, id={d.get(\"Entries\",[{}])[0].get(\"EventId\",\"?\")}')"

echo ""
echo "═══ ALL DONE ═══"
echo "Test locally:"
echo "  curl -4 -s -X POST \"$EP/2015-03-31/functions/$LAMBDA_NAME/invocations\" -H \"Content-Type: application/json\" -d '{\"httpMethod\":\"GET\",\"path\":\"/health\",\"headers\":{}}'"
echo ""
echo "Cleanup:"
echo "  podman stop localstack && pkill -f \"podman system service\" && rm -f /tmp/docker.sock"
